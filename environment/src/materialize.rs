//! Restore certified dependency archives into their original project layout.
//! Call only after verifying the image certificate and attached checkout lock.
use crate::BackendError;
use crate::preparation::{SEED_ROOT, unique_outputs};
use ayni_core::{DependencyPreparationPlan, PreparationOutputMode};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

mod archive;
#[cfg(test)]
mod tests;
use archive::{invalid, relative};

fn plans() -> io::Result<Vec<DependencyPreparationPlan>> {
    let path = Path::new(SEED_ROOT).join("plans.json");
    if !path.exists() {
        return Ok(Vec::new());
    }
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

fn groups(
    plans: &[DependencyPreparationPlan],
) -> io::Result<Vec<crate::preparation_groups::PreparationGroup>> {
    crate::preparation_groups::groups(plans).map_err(|e| io::Error::other(e.message))
}

pub fn validate_build() -> Result<(), BackendError> {
    validate(&plans().map_err(failure)?, None).map_err(failure)
}

fn validate(plans: &[DependencyPreparationPlan], checkout: Option<&Path>) -> io::Result<()> {
    let outputs = unique_outputs(plans);
    validate_outputs(&outputs)?;
    let mut links = BTreeMap::new();
    for group in groups(plans)? {
        let cache = archive::inspect(
            &Path::new(SEED_ROOT).join(&group.id).join("cache.tar"),
            Path::new("/home/ayni/.cache"),
        )?;
        for path in cache.links.keys() {
            archive::resolve(path, &cache.links, None)?;
        }
        let index = archive::inspect(
            &Path::new(SEED_ROOT).join(group.id).join("outputs.tar"),
            Path::new(crate::preparation::INPUT_ROOT),
        )?;
        for path in index.paths {
            if !outputs.iter().any(|o| path.starts_with(&o.path)) {
                return Err(invalid(
                    "archive entry is outside declared dependency outputs",
                ));
            }
        }
        for (path, target) in index.links {
            if links.insert(path, target).is_some() {
                return Err(invalid("duplicate dependency link"));
            }
        }
    }
    for path in links.keys() {
        archive::resolve(path, &links, checkout)?;
    }
    Ok(())
}

fn validate_outputs(outputs: &[ayni_core::PreparationOutput]) -> io::Result<()> {
    for (index, output) in outputs.iter().enumerate() {
        let path = relative(Path::new(&output.path))?;
        if path.as_os_str().is_empty() || output.path != output.mount_path {
            return Err(invalid(
                "dependency outputs must retain non-root project-relative locations",
            ));
        }
        if outputs[index + 1..]
            .iter()
            .any(|other| path.starts_with(&other.path) || Path::new(&other.path).starts_with(&path))
        {
            return Err(invalid("overlapping dependency outputs"));
        }
    }
    Ok(())
}

pub fn prepare(root: &Path, fingerprint: &str) -> Result<BTreeMap<String, String>, BackendError> {
    prepare_inner(root, fingerprint).map_err(failure)
}

fn prepare_inner(root: &Path, fingerprint: &str) -> io::Result<BTreeMap<String, String>> {
    let (state, _guard) = lock_preparation(root)?;
    let plans = plans()?;
    validate(&plans, Some(root))?;
    let cache = cache_directory()?;
    let environment = execution_environment(&plans, root, &cache);
    for group in groups(&plans)? {
        let seed = Path::new(SEED_ROOT).join(&group.id);
        restore_cache(&seed.join("cache.tar"), &cache, &environment)?;
        let outputs = unique_outputs(&group.plans);
        let marker = state.join(&group.id);
        let identity = format!("{fingerprint}\n");
        if prepared_marker(&marker, &identity)? {
            continue;
        }
        install_outputs(root, &seed, &outputs)?;
        run_materialization(&group.plans, root, &cache, &environment)?;
        use std::io::Write;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(marker)?
            .write_all(identity.as_bytes())?;
    }
    Ok(environment)
}

fn cache_directory() -> io::Result<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("ayni-cache-{}", unsafe { libc::geteuid() }))
        });
    safe_directory(&cache)?;
    cache.canonicalize()
}

fn lock_preparation(root: &Path) -> io::Result<(PathBuf, fs::File)> {
    use std::os::unix::fs::OpenOptionsExt;
    let state = root.join(".ayni/prepared");
    safe_directory(&state)?;
    let guard = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(state.join("lock"))?;
    if !guard.metadata()?.is_file() {
        return Err(invalid("preparation lock is not a regular file"));
    }
    guard.lock()?;
    Ok((state, guard))
}

fn prepared_marker(path: &Path, identity: &str) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
        Ok(metadata) if metadata.is_file() && metadata.len() == identity.len() as u64 => {
            use std::io::Read;
            use std::os::unix::fs::OpenOptionsExt;
            let mut value = String::new();
            fs::OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)?
                .take(256)
                .read_to_string(&mut value)?;
            if value == identity {
                Ok(true)
            } else {
                Err(invalid(
                    "prepared checkout belongs to a different environment",
                ))
            }
        }
        Ok(_) => Err(invalid(
            "invalid preparation marker; existing outputs were not replaced",
        )),
    }
}

fn install_outputs(
    root: &Path,
    seed: &Path,
    outputs: &[ayni_core::PreparationOutput],
) -> io::Result<()> {
    for output in outputs {
        if output.path != output.mount_path
            || relative(Path::new(&output.path))?.as_os_str().is_empty()
        {
            return Err(invalid(
                "dependency outputs must retain their project-relative locations",
            ));
        }
        let destination = root.join(&output.path);
        safe_directory(destination.parent().unwrap())?;
        if fs::symlink_metadata(&destination).is_ok() {
            return Err(invalid(format!(
                "refusing to replace existing dependency output: {}",
                output.path
            )));
        }
    }
    let staging = tempfile::tempdir_in(root)?;
    archive::unpack(
        &seed.join("outputs.tar"),
        staging.path(),
        Path::new(crate::preparation::INPUT_ROOT),
    )?;
    for output in outputs {
        let source = staging.path().join(&output.path);
        if output.mode == PreparationOutputMode::Fresh {
            fs::create_dir_all(&source)?;
        }
        fs::rename(source, root.join(&output.path))?;
    }
    Ok(())
}

fn run_materialization(
    plans: &[DependencyPreparationPlan],
    root: &Path,
    cache: &Path,
    environment: &BTreeMap<String, String>,
) -> io::Result<()> {
    let mut completed = Vec::new();
    for plan in plans {
        for command in &plan.materialization_commands {
            if completed.contains(command) {
                continue;
            }
            let mut args = environment
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>();
            args.extend(
                command
                    .environment
                    .iter()
                    .map(|(k, v)| format!("{k}={}", relocate(v, root, cache))),
            );
            args.push(command.program.clone());
            args.extend(command.args.clone());
            let result = crate::run_oci_command(
                &root.join(&command.cwd),
                "env",
                &args,
                Duration::from_secs(600),
            )
            .map_err(io::Error::other)?;
            if !result.status.success() {
                return Err(invalid(format!(
                    "dependency materialization failed: {}",
                    crate::concise_output(&result.stderr)
                )));
            }
            completed.push(command.clone());
        }
    }
    Ok(())
}

fn relocate(value: &str, root: &Path, cache: &Path) -> String {
    for (prefix, target) in [("/home/ayni/.cache", cache), ("/workspace", root)] {
        if let Ok(suffix) = Path::new(value).strip_prefix(prefix) {
            return target.join(suffix).to_string_lossy().into_owned();
        }
    }
    value.into()
}

fn execution_environment(
    plans: &[DependencyPreparationPlan],
    root: &Path,
    cache: &Path,
) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::from([
        (
            "XDG_CACHE_HOME".into(),
            cache.to_string_lossy().into_owned(),
        ),
        (
            "CARGO_HOME".into(),
            cache.join("cargo").to_string_lossy().into_owned(),
        ),
        (
            "npm_config_cache".into(),
            cache.join("npm").to_string_lossy().into_owned(),
        ),
    ]);
    for plan in plans {
        for (key, value) in &plan.execution_environment {
            environment.insert(key.clone(), relocate(value, root, cache));
        }
    }
    for key in [
        "CARGO_HOME",
        "CARGO_TARGET_DIR",
        "npm_config_cache",
        "UV_CACHE_DIR",
        "GRADLE_USER_HOME",
        "GOCACHE",
        "GOMODCACHE",
    ] {
        if let Ok(value) = std::env::var(key)
            && !value.starts_with("/home/ayni/.cache")
        {
            environment.insert(key.into(), value);
        }
    }
    environment
}

fn restore_cache(
    archive_path: &Path,
    cache: &Path,
    environment: &BTreeMap<String, String>,
) -> io::Result<()> {
    let index = archive::inspect(archive_path, Path::new("/home/ayni/.cache"))?;
    for path in index.links.keys() {
        archive::resolve(path, &index.links, None)?;
    }
    let staging = tempfile::tempdir_in(cache)?;
    archive::unpack(archive_path, staging.path(), Path::new("/home/ayni/.cache"))?;
    copy_missing(staging.path(), cache)?;
    for (name, variable) in [
        ("cargo", "CARGO_HOME"),
        ("npm", "npm_config_cache"),
        ("uv", "UV_CACHE_DIR"),
        ("gradle", "GRADLE_USER_HOME"),
        ("go/build", "GOCACHE"),
        ("go/pkg/mod", "GOMODCACHE"),
    ] {
        if let Some(target) = environment.get(variable) {
            let source = cache.join(name);
            if source.is_dir() && Path::new(target) != source {
                copy_cache(&source, Path::new(target))?;
            }
        }
    }
    Ok(())
}

fn copy_cache(source: &Path, destination: &Path) -> io::Result<()> {
    if destination.starts_with(source) {
        return Err(invalid("cache destination overlaps its seed"));
    }
    safe_directory(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_cache(&entry.path(), &target)?;
        } else if fs::symlink_metadata(&target).is_err() {
            copy_cache_entry(&entry, &target)?;
        }
    }
    Ok(())
}

fn copy_cache_entry(entry: &fs::DirEntry, target: &Path) -> io::Result<()> {
    if entry.file_type()?.is_symlink() {
        // The complete cache layout remains at its canonical root. A caller
        // may relocate one cache subtree; retain links to that complete layout
        // rather than interpreting sibling-relative targets at the new root.
        let original = entry.path();
        let link = original.parent().unwrap().join(fs::read_link(&original)?);
        std::os::unix::fs::symlink(link, target)?;
    } else {
        fs::copy(entry.path(), target)?;
    }
    Ok(())
}

fn copy_missing(source: &Path, destination: &Path) -> io::Result<()> {
    safe_directory(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_missing(&entry.path(), &target)?;
        } else if fs::symlink_metadata(&target).is_err() {
            fs::rename(entry.path(), target)?;
        }
    }
    Ok(())
}

fn safe_directory(path: &Path) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Ok(());
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            if let Some(parent) = path.parent() {
                safe_directory(parent)?;
            }
            Ok(())
        }
        Ok(_) => Err(invalid(format!(
            "directory is a link or non-directory: {}",
            path.display()
        ))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path.parent() {
                safe_directory(parent)?;
            }
            fs::create_dir(path)
        }
        Err(e) => Err(e),
    }
}

fn failure(error: io::Error) -> BackendError {
    BackendError::environment(format!("dependency preparation: {error}"))
}
