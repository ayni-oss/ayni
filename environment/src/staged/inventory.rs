//! Host-side bounded inventory of an immutable extracted filesystem.
use crate::BackendError;
use std::path::Path;

pub(super) fn inventory(root: &Path) -> Result<String, BackendError> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::inventory(root, unix::Limits::default())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = root;
        Err(BackendError::environment(
            "staged inventory requires Linux or macOS descriptor-relative filesystem access",
        ))
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use super::*;
    use crate::certificate::{ProtectedManifestEntry, parse_protected_manifest, render_manifest};
    use sha2::{Digest, Sha256};
    use std::{
        ffi::{CStr, CString},
        fs::{File, Metadata},
        io::Read,
        os::{
            fd::{AsRawFd, FromRawFd, RawFd},
            unix::{ffi::OsStrExt, fs::MetadataExt},
        },
    };
    #[derive(Clone, Copy)]
    pub(super) struct Limits {
        file: u64,
        total: u64,
        entries: usize,
        depth: usize,
        manifest: u64,
    }
    impl Default for Limits {
        fn default() -> Self {
            Self {
                file: 256 * 1024 * 1024,
                total: 8 * 1024 * 1024 * 1024,
                entries: crate::MAX_PROTECTED_MANIFEST_ENTRIES,
                depth: 128,
                manifest: crate::MAX_PROTECTED_MANIFEST_BYTES,
            }
        }
    }
    struct Inventory {
        limits: Limits,
        total: u64,
        nodes: usize,
        remaining_names: usize,
        bytes: u64,
        entries: Vec<ProtectedManifestEntry>,
    }
    fn failure(message: &str) -> BackendError {
        BackendError::environment(format!("staged protected-content inventory: {message}"))
    }
    fn cstring(value: &[u8]) -> Result<CString, BackendError> {
        CString::new(value).map_err(|_| failure("path contains a NUL byte"))
    }
    fn directory(parent: RawFd, name: &CStr) -> Result<File, std::io::Error> {
        // SAFETY: name is NUL-terminated; openat returns a newly owned descriptor.
        let fd = unsafe {
            libc::openat(
                parent,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            // SAFETY: successful openat transfers sole ownership of fd to File.
            Ok(unsafe { File::from_raw_fd(fd) })
        }
    }
    fn metadata(parent: RawFd, name: &CStr) -> Result<libc::stat, BackendError> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: pointers are valid; successful fstatat initializes stat without following links.
        if unsafe {
            libc::fstatat(
                parent,
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(failure("cannot inspect entry metadata"));
        }
        // SAFETY: fstatat succeeded and initialized the complete stat structure.
        Ok(unsafe { stat.assume_init() })
    }
    fn same(a: &Metadata, b: &Metadata) -> bool {
        a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.mode() == b.mode()
            && a.uid() == b.uid()
            && a.gid() == b.gid()
            && a.len() == b.len()
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
            && a.nlink() == b.nlink()
    }
    fn identified(stat: &libc::stat, meta: &Metadata) -> bool {
        #[cfg(target_os = "linux")]
        let (device, mode) = (stat.st_dev, stat.st_mode);
        #[cfg(target_os = "macos")]
        let (device, mode) = (stat.st_dev as u64, u32::from(stat.st_mode));
        device == meta.dev() && stat.st_ino == meta.ino() && mode == meta.mode()
    }
    fn errno() -> *mut libc::c_int {
        #[cfg(target_os = "linux")]
        // SAFETY: libc exposes this thread's errno location.
        {
            unsafe { libc::__errno_location() }
        }
        #[cfg(target_os = "macos")]
        // SAFETY: libc exposes this thread's errno location.
        {
            unsafe { libc::__error() }
        }
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory {
        fn drop(&mut self) {
            // SAFETY: this object exclusively owns the DIR returned by fdopendir.
            unsafe { libc::closedir(self.0) };
        }
    }
    fn names(file: &File, budget: &mut usize) -> Result<Vec<Vec<u8>>, BackendError> {
        // Open a fresh descriptor to avoid sharing directory offsets across traversals.
        let dot = cstring(b".")?;
        let owned =
            directory(file.as_raw_fd(), &dot).map_err(|_| failure("cannot reopen directory"))?;
        use std::os::fd::IntoRawFd;
        let fd = owned.into_raw_fd();
        // SAFETY: fd is newly owned and references a directory.
        let raw = unsafe { libc::fdopendir(fd) };
        if raw.is_null() {
            // SAFETY: failed fdopendir did not take ownership of fd.
            unsafe { libc::close(fd) };
            return Err(failure("cannot enumerate directory"));
        }
        let dir = Directory(raw);
        let mut names = Vec::new();
        loop {
            // SAFETY: errno is thread-local and dir remains live throughout readdir.
            unsafe { *errno() = 0 };
            // SAFETY: dir exclusively owns a valid DIR; entries are copied before next call.
            let entry = unsafe { libc::readdir(dir.0) };
            if entry.is_null() {
                // SAFETY: errno remains valid for this thread.
                if unsafe { *errno() } != 0 {
                    return Err(failure("directory enumeration failed"));
                }
                break;
            }
            // SAFETY: readdir returns a NUL-terminated name valid until its next call.
            let name = unsafe { CStr::from_ptr((*entry).d_name.as_ptr()) }.to_bytes();
            if name == b"." || name == b".." {
                continue;
            }
            // Charge names when discovered, including pending siblings retained
            // while descending. Every directory shares this allocation budget.
            *budget = budget
                .checked_sub(1)
                .ok_or_else(|| failure("entry limit exceeded"))?;
            names.push(name.to_vec());
        }
        names.sort();
        Ok(names)
    }
    impl Inventory {
        fn push(&mut self, entry: ProtectedManifestEntry) -> Result<(), BackendError> {
            if self.entries.len() >= self.limits.entries {
                return Err(failure("entry limit exceeded"));
            }
            let bytes = serde_json::to_vec(&entry)
                .map_err(|_| failure("cannot encode manifest entry"))?
                .len() as u64
                + 1;
            self.bytes = self
                .bytes
                .checked_add(bytes)
                .ok_or_else(|| failure("manifest limit exceeded"))?;
            if self.bytes > self.limits.manifest {
                return Err(failure("manifest limit exceeded"));
            }
            self.entries.push(entry);
            Ok(())
        }
        fn visit(
            &mut self,
            parent: &File,
            name: &CStr,
            path: &str,
            depth: usize,
        ) -> Result<(), BackendError> {
            self.nodes += 1;
            if self.nodes > self.limits.entries {
                return Err(failure("entry limit exceeded"));
            }
            if depth > self.limits.depth {
                return Err(failure("directory depth limit exceeded"));
            }
            let stat = metadata(parent.as_raw_fd(), name)?;
            match stat.st_mode & libc::S_IFMT {
                libc::S_IFREG => self.file(parent, name, path, &stat),
                libc::S_IFDIR => self.tree(parent, name, path, depth, &stat),
                libc::S_IFLNK => self.symlink(parent, name, path, &stat),
                _ => Err(failure("special files are not supported")),
            }
        }
        fn tree(
            &mut self,
            parent: &File,
            name: &CStr,
            path: &str,
            depth: usize,
            stat: &libc::stat,
        ) -> Result<(), BackendError> {
            let file = directory(parent.as_raw_fd(), name)
                .map_err(|_| failure("cannot open directory without following links"))?;
            let before = file
                .metadata()
                .map_err(|_| failure("cannot inspect open directory"))?;
            if !identified(stat, &before) {
                return Err(failure("directory changed during inspection"));
            }
            for child in names(&file, &mut self.remaining_names)? {
                let child_path = format!(
                    "{path}/{}",
                    std::str::from_utf8(&child).map_err(|_| failure("path is not UTF-8"))?
                );
                self.visit(&file, &cstring(&child)?, &child_path, depth + 1)?;
            }
            let after = file
                .metadata()
                .map_err(|_| failure("cannot inspect open directory"))?;
            if !same(&before, &after) || !identified(&metadata(parent.as_raw_fd(), name)?, &after) {
                return Err(failure("directory changed during inspection"));
            }
            Ok(())
        }
        fn symlink(
            &mut self,
            parent: &File,
            name: &CStr,
            path: &str,
            stat: &libc::stat,
        ) -> Result<(), BackendError> {
            let mut bytes = vec![0_u8; 65_536];
            // SAFETY: destination is writable; readlinkat does not follow the target.
            let length = unsafe {
                libc::readlinkat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    bytes.as_mut_ptr().cast(),
                    bytes.len(),
                )
            };
            if length < 0 || length as usize >= bytes.len() {
                return Err(failure("cannot read bounded symlink target"));
            }
            bytes.truncate(length as usize);
            if !same_symlink(stat, &metadata(parent.as_raw_fd(), name)?) {
                return Err(failure("symlink changed during inspection"));
            }
            self.push(ProtectedManifestEntry::Symlink {
                path: path.into(),
                target: String::from_utf8(bytes)
                    .map_err(|_| failure("symlink target is not UTF-8"))?,
            })
        }
        fn check_size(&self, length: u64) -> Result<(), BackendError> {
            if length > self.limits.file || length > self.limits.total.saturating_sub(self.total) {
                return Err(failure("file or total byte limit exceeded"));
            }
            Ok(())
        }
        fn hash(&self, file: &mut File) -> Result<(u64, String), BackendError> {
            let mut hash = Sha256::new();
            let mut bytes = 0_u64;
            let mut buffer = [0_u8; 65_536];
            loop {
                let read = file
                    .read(&mut buffer)
                    .map_err(|_| failure("cannot read protected file"))?;
                if read == 0 {
                    break;
                }
                bytes += read as u64;
                self.check_size(bytes)?;
                hash.update(&buffer[..read]);
            }
            Ok((
                bytes,
                format!("sha256:{}", ayni_core::lower_hex(hash.finalize())),
            ))
        }
        fn file(
            &mut self,
            parent: &File,
            name: &CStr,
            path: &str,
            stat: &libc::stat,
        ) -> Result<(), BackendError> {
            // SAFETY: name is valid and flags prevent following links or blocking on replaced special files.
            let fd = unsafe {
                libc::openat(
                    parent.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                return Err(failure("cannot open regular file without following links"));
            }
            // SAFETY: successful openat transferred a newly owned descriptor.
            let mut file = unsafe { File::from_raw_fd(fd) };
            let before = file
                .metadata()
                .map_err(|_| failure("cannot inspect open file"))?;
            if !before.is_file() || !identified(stat, &before) {
                return Err(failure("file changed during inspection"));
            }
            self.check_size(before.len())?;
            let (bytes, sha256) = self.hash(&mut file)?;
            let after = file
                .metadata()
                .map_err(|_| failure("cannot inspect open file"))?;
            if bytes != before.len()
                || !same(&before, &after)
                || !identified(&metadata(parent.as_raw_fd(), name)?, &after)
            {
                return Err(failure("file changed during inspection"));
            }
            self.total += bytes;
            self.push(ProtectedManifestEntry::File {
                path: path.into(),
                sha256,
            })
        }
    }
    fn same_symlink(before: &libc::stat, after: &libc::stat) -> bool {
        before.st_dev == after.st_dev
            && before.st_ino == after.st_ino
            && before.st_mode == after.st_mode
            && before.st_size == after.st_size
            && before.st_mtime == after.st_mtime
            && before.st_mtime_nsec == after.st_mtime_nsec
            && before.st_ctime == after.st_ctime
            && before.st_ctime_nsec == after.st_ctime_nsec
    }
    fn parent_directory(
        base: &File,
        components: &[&str],
        optional: bool,
    ) -> Result<Option<File>, BackendError> {
        let mut parent = base
            .try_clone()
            .map_err(|_| failure("cannot duplicate filesystem root"))?;
        for component in components {
            match directory(parent.as_raw_fd(), &cstring(component.as_bytes())?) {
                Ok(next) => parent = next,
                Err(error) if optional && error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(None);
                }
                Err(_) => return Err(failure("protected ancestor must be an actual directory")),
            }
        }
        Ok(Some(parent))
    }
    fn entry_exists(parent: &File, name: &CStr) -> Result<bool, BackendError> {
        let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
        // SAFETY: fstatat observes this descriptor-relative entry without following links.
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                name.as_ptr(),
                stat.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0
        {
            return Ok(true);
        }
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            return Ok(false);
        }
        Err(failure("cannot inspect optional protected root"))
    }
    impl Inventory {
        fn root(&mut self, base: &File, logical: &str, optional: bool) -> Result<(), BackendError> {
            let components = logical
                .trim_start_matches('/')
                .split('/')
                .collect::<Vec<_>>();
            let Some(parent) =
                parent_directory(base, &components[..components.len() - 1], optional)?
            else {
                return Ok(());
            };
            let name = cstring(
                components
                    .last()
                    .ok_or_else(|| failure("invalid protected root"))?
                    .as_bytes(),
            )?;
            if optional && !entry_exists(&parent, &name)? {
                return Ok(());
            }
            self.visit(&parent, &name, logical, components.len())
        }
    }
    pub(super) fn inventory(root: &Path, limits: Limits) -> Result<String, BackendError> {
        // Remove trailing separators and dot components so O_NOFOLLOW applies to
        // the selected directory rather than an implicit final `.` lookup.
        let normalized = root.components().collect::<std::path::PathBuf>();
        let root_name = cstring(normalized.as_os_str().as_bytes())?;
        let base = directory(libc::AT_FDCWD, &root_name)
            .map_err(|_| failure("filesystem root must be an actual directory"))?;
        let base_before = base
            .metadata()
            .map_err(|_| failure("cannot inspect filesystem root"))?;
        let mut inventory = Inventory {
            limits,
            total: 0,
            nodes: 0,
            remaining_names: limits.entries,
            bytes: render_manifest(&[]).len() as u64,
            entries: Vec::new(),
        };
        for logical in crate::PROTECTED_FILE_ROOTS {
            inventory.root(&base, logical, false)?;
        }
        for logical in crate::PROTECTED_TREE_ROOTS {
            inventory.root(&base, logical, true)?;
        }
        inventory
            .entries
            .sort_by(|a, b| a.path().cmp(b.path()).then(a.cmp(b)));
        let base_after = base
            .metadata()
            .map_err(|_| failure("cannot inspect filesystem root"))?;
        if !same(&base_before, &base_after) {
            return Err(failure("filesystem root changed during inspection"));
        }
        let manifest = render_manifest(&inventory.entries);
        parse_protected_manifest(&manifest)?;
        Ok(manifest)
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::fs;
        use std::os::unix::fs::symlink;
        fn fixture() -> tempfile::TempDir {
            let root = tempfile::tempdir().unwrap();
            for path in crate::PROTECTED_FILE_ROOTS {
                let actual = root.path().join(path.trim_start_matches('/'));
                fs::create_dir_all(actual.parent().unwrap()).unwrap();
                fs::write(actual, b"fixture content").unwrap();
            }
            fs::create_dir_all(root.path().join("opt/ayni/mise/installs")).unwrap();
            fs::write(
                root.path().join("opt/ayni/mise/installs/tool"),
                b"installed fixture tool",
            )
            .unwrap();
            root
        }
        #[test]
        fn canonical_inventory_hashes_actual_bytes_and_additional_protected_files() {
            let root = fixture();
            let original = inventory(root.path(), Limits::default()).unwrap();
            assert_eq!(original, inventory(root.path(), Limits::default()).unwrap());
            parse_protected_manifest(&original).unwrap();
            fs::write(root.path().join("usr/local/bin/ayni"), b"tampered bytes").unwrap();
            let changed = inventory(root.path(), Limits::default()).unwrap();
            assert_ne!(original, changed);
            fs::create_dir_all(root.path().join("opt/ayni/tools")).unwrap();
            fs::write(root.path().join("opt/ayni/tools/extra"), b"unlisted tool").unwrap();
            let extra = inventory(root.path(), Limits::default()).unwrap();
            assert_ne!(changed, extra);
            assert!(extra.contains("/opt/ayni/tools/extra"));
        }
        #[test]
        fn ancestor_symlinks_and_external_targets_fail_closed() {
            let root = fixture();
            let external = tempfile::tempdir().unwrap();
            fs::create_dir_all(root.path().join("home/ayni")).unwrap();
            symlink(external.path(), root.path().join("home/ayni/.cache")).unwrap();
            assert!(inventory(root.path(), Limits::default()).is_err());
            fs::remove_file(root.path().join("home/ayni/.cache")).unwrap();
            fs::create_dir_all(root.path().join("opt/ayni")).unwrap();
            symlink("/etc/passwd", root.path().join("opt/ayni/outside")).unwrap();
            assert!(inventory(root.path(), Limits::default()).is_err());
        }
        #[test]
        fn confined_symlinks_are_recorded_without_following_them() {
            let root = fixture();
            fs::create_dir_all(root.path().join("opt/ayni")).unwrap();
            symlink("/usr/local/bin/ayni", root.path().join("opt/ayni/tool")).unwrap();
            let manifest = inventory(root.path(), Limits::default()).unwrap();
            assert!(manifest.contains("\"target\":\"/usr/local/bin/ayni\""));
        }
        #[test]
        fn file_total_entry_depth_and_manifest_bounds_are_enforced() {
            let root = fixture();
            let normal = Limits::default();
            for limits in [
                Limits { file: 1, ..normal },
                Limits { total: 1, ..normal },
                Limits {
                    entries: 1,
                    ..normal
                },
                Limits { depth: 2, ..normal },
                Limits {
                    manifest: 1,
                    ..normal
                },
            ] {
                assert!(inventory(root.path(), limits).is_err());
            }
            // A sparse oversized file is rejected from metadata, before reading its content.
            File::options()
                .write(true)
                .open(root.path().join("usr/local/bin/ayni"))
                .unwrap()
                .set_len(normal.file + 1)
                .unwrap();
            assert!(inventory(root.path(), normal).is_err());
        }
        #[test]
        fn nested_wide_directories_share_one_pending_name_allocation_budget() {
            let root = tempfile::tempdir().unwrap();
            let nested = root.path().join("a");
            fs::create_dir(&nested).unwrap();
            for index in 0..4 {
                fs::write(root.path().join(format!("z{index}")), b"").unwrap();
                fs::write(nested.join(format!("z{index}")), b"").unwrap();
            }
            let parent = File::open(root.path()).unwrap();
            let child = File::open(&nested).unwrap();
            let mut budget = 8;
            let pending = names(&parent, &mut budget).unwrap();
            assert_eq!(pending.len(), 5);
            assert_eq!(budget, 3);
            // The five parent names are still allocated and must not be forgotten
            // when descending into the lexicographically first child directory.
            assert!(names(&child, &mut budget).is_err());
            assert_eq!(budget, 0);
            assert_eq!(pending.len(), 5);
        }
        #[test]
        fn special_files_and_symlink_base_are_rejected() {
            let root = fixture();
            fs::create_dir_all(root.path().join("opt/ayni")).unwrap();
            let fifo = cstring(root.path().join("opt/ayni/fifo").as_os_str().as_bytes()).unwrap();
            // SAFETY: the synthetic path is NUL terminated and mkfifo creates no external process.
            assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
            assert!(inventory(root.path(), Limits::default()).is_err());
            let holder = tempfile::tempdir().unwrap();
            symlink(root.path(), holder.path().join("link")).unwrap();
            assert!(inventory(&holder.path().join("link"), Limits::default()).is_err());
        }
    }
}
