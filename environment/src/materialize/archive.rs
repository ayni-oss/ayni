//! Inspect archive paths and links before creating anything in a checkout.
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

pub(super) fn relative(path: &Path) -> io::Result<PathBuf> {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => (),
            Component::Normal(value) => result.push(value),
            Component::ParentDir if result.pop() => (),
            _ => return Err(invalid(format!("path escapes project: {}", path.display()))),
        }
    }
    Ok(result)
}

pub(super) struct Index {
    pub links: BTreeMap<PathBuf, PathBuf>,
    pub paths: Vec<PathBuf>,
}

fn entry_path(entry: &tar::Entry<'_, fs::File>) -> io::Result<PathBuf> {
    let original = entry.path()?.into_owned();
    let path = relative(&original)?;
    if original.components().any(|c| c == Component::ParentDir) {
        return Err(invalid("archive entry contains parent traversal"));
    }
    let kind = entry.header().entry_type();
    if !(kind.is_dir() || kind.is_file() || kind.is_symlink()) {
        return Err(invalid("unsupported archive entry type"));
    }
    Ok(path)
}

fn entry_target(
    entry: &tar::Entry<'_, fs::File>,
    path: &Path,
    original_root: &Path,
) -> io::Result<PathBuf> {
    let target = entry
        .link_name()?
        .ok_or_else(|| invalid("missing link target"))?;
    let target = portable_target(path, &target, original_root)?;
    relative(&path.parent().unwrap_or(Path::new("")).join(&target))?;
    Ok(target)
}

pub(super) fn inspect(path: &Path, original_root: &Path) -> io::Result<Index> {
    let mut archive = tar::Archive::new(fs::File::open(path)?);
    let mut links = BTreeMap::new();
    let mut paths = BTreeMap::new();
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry_path(&entry)?;
        let kind = entry.header().entry_type();
        if paths.insert(path.clone(), kind).is_some() {
            return Err(invalid("duplicate archive entry"));
        }
        if kind.is_symlink() {
            let target = entry_target(&entry, &path, original_root)?;
            links.insert(path, target);
        }
    }
    for path in paths.keys() {
        if path.ancestors().skip(1).any(|p| links.contains_key(p)) {
            return Err(invalid("archive writes through a symlink"));
        }
    }
    Ok(Index {
        links,
        paths: paths.into_keys().collect(),
    })
}

/// Resolve every hop, including existing source-directory symlinks. Lexical
/// normalization alone would miss an intermediate directory that escapes.
pub(super) fn resolve(
    path: &Path,
    links: &BTreeMap<PathBuf, PathBuf>,
    checkout: Option<&Path>,
) -> io::Result<PathBuf> {
    let mut pending = path.to_path_buf();
    for _ in 0..64 {
        let mut prefix = PathBuf::new();
        let mut replaced = false;
        let current = pending.clone();
        let parts = current.components().collect::<Vec<_>>();
        for (index, part) in parts.iter().enumerate() {
            match part {
                Component::CurDir => continue,
                Component::Normal(value) => prefix.push(value),
                Component::ParentDir if prefix.pop() => continue,
                _ => return Err(invalid("symlink chain escapes project")),
            }
            let target = match links.get(&prefix) {
                Some(value) => Some(value.clone()),
                None => checkout_link(checkout, &prefix)?,
            };
            if let Some(target) = target {
                pending = next_link(&prefix, &target, checkout)?;
                pending.extend(parts[index + 1..].iter().map(|part| part.as_os_str()));
                replaced = true;
                break;
            }
        }
        if !replaced {
            return Ok(prefix);
        }
    }
    Err(invalid("cyclic or excessively deep symlink chain"))
}

fn next_link(prefix: &Path, target: &Path, checkout: Option<&Path>) -> io::Result<PathBuf> {
    if !target.is_absolute() {
        return Ok(prefix.parent().unwrap_or(Path::new("")).join(target));
    }
    let root = checkout.ok_or_else(|| invalid("absolute symlink in dependency archive"))?;
    target
        .strip_prefix(root)
        .map(Path::to_path_buf)
        .map_err(|_| invalid("source symlink escapes project"))
}

fn checkout_link(checkout: Option<&Path>, prefix: &Path) -> io::Result<Option<PathBuf>> {
    match checkout.map(|root| fs::read_link(root.join(prefix))) {
        Some(Ok(value)) => Ok(Some(value)),
        Some(Err(e))
            if !matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
            ) =>
        {
            Err(e)
        }
        _ => Ok(None),
    }
}

fn portable_target(path: &Path, target: &Path, original_root: &Path) -> io::Result<PathBuf> {
    if target.as_os_str().is_empty() {
        return Err(invalid("empty symlink target"));
    }
    if !target.is_absolute() {
        return Ok(target.to_path_buf());
    }
    let suffix = target
        .strip_prefix(original_root)
        .map_err(|_| invalid("absolute symlink escapes prepared root"))?;
    let mut result = PathBuf::new();
    for _ in path.parent().unwrap_or(Path::new("")).components() {
        result.push("..");
    }
    result.push(relative(suffix)?);
    Ok(result)
}

pub(super) fn unpack(path: &Path, destination: &Path, original_root: &Path) -> io::Result<()> {
    let mut archive = tar::Archive::new(fs::File::open(path)?);
    archive.set_preserve_permissions(false);
    archive.set_preserve_ownerships(false);
    for entry in archive.entries()? {
        let mut entry = entry?;
        if entry.header().entry_type().is_symlink() {
            let path = relative(&entry.path()?)?;
            let target = entry_target(&entry, &path, original_root)?;
            if let Some(parent) = destination.join(&path).parent() {
                fs::create_dir_all(parent)?;
            }
            std::os::unix::fs::symlink(target, destination.join(path))?;
        } else if !entry.unpack_in(destination)? {
            return Err(invalid("archive entry escaped destination"));
        }
    }
    Ok(())
}
