//! Lock-driven OCI image construction and target activation.
//!
//! Ayni builds images but never launches quality commands itself. Platforms
//! attach a checkout and invoke the image entrypoint using their own controls.

use crate::BackendError;
use ayni_core::{ArtifactToolVersion, EnvironmentLock, LockedTargetEnvironment};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub const WORKSPACE: &str = "/workspace";

mod engine;
pub use engine::{
    BuildCache, Engine, build, build_prepared, build_prepared_with_cache,
    build_prepared_with_executor, detect_engine, doctor, doctor_prepared,
};

pub fn locked_tool_versions(lock: &EnvironmentLock) -> Vec<ArtifactToolVersion> {
    let mut versions = vec![ArtifactToolVersion {
        tool: String::from("mise"),
        version: lock.mise_version().to_owned(),
    }];
    for target in lock.targets() {
        let prefix = format!("{}:{}", target.target.language.as_str(), target.target.root);
        versions.extend(target.runtimes.iter().map(|runtime| ArtifactToolVersion {
            tool: format!("{prefix}:runtime:{}", runtime.runtime),
            version: runtime.version.clone(),
        }));
        if let Some(manager) = &target.package_manager {
            versions.push(ArtifactToolVersion {
                tool: format!("{prefix}:package_manager:{}", manager.family),
                version: manager.version.clone(),
            });
        }
    }
    versions.sort_by(|left, right| left.tool.cmp(&right.tool));
    versions
}

pub fn target_environment(
    target: &LockedTargetEnvironment,
) -> Result<Vec<(String, String)>, BackendError> {
    let mut variables = BTreeMap::new();
    for runtime in &target.runtimes {
        variables.insert(
            mise_version_variable(&runtime.runtime)?,
            runtime.version.clone(),
        );
        if runtime.runtime == "java" {
            validate_mise_install_version("java", &runtime.version)?;
            variables.insert(
                String::from("JAVA_HOME"),
                format!("/opt/ayni/mise/installs/java/{}", runtime.version),
            );
        }
    }
    if let Some(manager) = &target.package_manager
        && !matches!(manager.family.as_str(), "npm" | "pnpm" | "gradle")
    {
        variables.insert(
            mise_version_variable(&manager.family)?,
            manager.version.clone(),
        );
    }
    Ok(variables.into_iter().collect())
}

fn mise_version_variable(tool: &str) -> Result<String, BackendError> {
    if tool.is_empty()
        || !tool
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(BackendError::environment(format!(
            "locked mise tool name cannot be activated safely: {tool}"
        )));
    }
    Ok(format!(
        "MISE_{}_VERSION",
        tool.to_ascii_uppercase().replace('-', "_")
    ))
}

fn validate_mise_install_version(tool: &str, version: &str) -> Result<(), BackendError> {
    let safe = !version.is_empty()
        && version != "."
        && version != ".."
        && version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'+'));
    if safe {
        Ok(())
    } else {
        Err(BackendError::environment(format!(
            "locked {tool} version cannot form a safe mise install path: {version}"
        )))
    }
}

pub(crate) fn ensure_managed_directory(path: &Path) -> Result<(), BackendError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
        Ok(_) => Err(BackendError::execution(format!(
            "managed environment state must not contain symlinks or non-directories: {}",
            path.display()
        ))),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|error| {
                BackendError::execution(format!(
                    "failed to create managed environment state {}: {error}",
                    path.display()
                ))
            })?;
            ensure_managed_directory(path)
        }
        Err(error) => Err(BackendError::execution(format!(
            "failed to inspect managed environment state {}: {error}",
            path.display()
        ))),
    }
}

pub(super) fn canonical_root(path: &Path) -> Result<PathBuf, BackendError> {
    let root = path.canonicalize().map_err(|error| {
        BackendError::input(format!(
            "failed to establish repository root {}: {error}",
            path.display()
        ))
    })?;
    if root.is_dir() {
        Ok(root)
    } else {
        Err(BackendError::input(format!(
            "repository root is not a directory: {}",
            root.display()
        )))
    }
}
