//! Bound every frozen source read before using the ordinary lock validator.
use crate::application_error::ApplicationError;
use ayni_adapters_common::repository::contained_existing_path;
use ayni_core::EnvironmentLock;
use std::{collections::BTreeSet, fs, io::Read, path::Path};

const LOCK_BYTES: u64 = 1024 * 1024;
const INPUT_BYTES: u64 = 512 * 1024;
const TOTAL_BYTES: u64 = 8 * 1024 * 1024;
const LOCK_FILE: &str = ".ayni.lock";

pub(super) fn validated_lock(root: &Path) -> Result<EnvironmentLock, ApplicationError> {
    let bytes = bounded_read(&root.join(LOCK_FILE), LOCK_BYTES)?;
    let lock: EnvironmentLock = serde_json::from_slice(&bytes).map_err(|_| {
        ApplicationError::environment("committed environment lock is invalid or unsupported")
    })?;
    let mut total = bytes.len() as u64;
    let paths = input_paths(&lock);
    if paths.len() + 1 > 256 {
        return Err(bounds_error());
    }
    for path in paths {
        let path = contained_existing_path(root, Path::new(&path)).map_err(|_| {
            ApplicationError::environment(
                "locked input is unavailable or escapes the frozen repository",
            )
        })?;
        let bytes = bounded_read(&path, INPUT_BYTES)?;
        total = total.saturating_add(bytes.len() as u64);
        if total > TOTAL_BYTES {
            return Err(bounds_error());
        }
    }
    // The caller holds this snapshot read-only. Every path the public validator
    // may read has now passed regular-file and size checks without running tools.
    ayni_environment::read_lock(root).map_err(ApplicationError::from)
}

fn input_paths(lock: &EnvironmentLock) -> BTreeSet<String> {
    let mut paths = BTreeSet::from([lock.repository().contract_path.clone()]);
    for target in lock.targets() {
        for source in target
            .runtimes
            .iter()
            .map(|value| &value.source)
            .chain(target.package_manager.iter().map(|value| &value.source))
            .chain(target.signal_tools.iter().map(|value| &value.source))
            .chain(target.dependency_locks.iter().map(|value| &value.source))
        {
            if source.digest.is_some() {
                paths.insert(source.path.clone());
            }
        }
        for dependency in &target.dependency_locks {
            paths.insert(dependency.path.clone());
        }
    }
    for source in lock
        .tools()
        .iter()
        .map(|value| &value.source)
        .chain(lock.debian_packages().iter().map(|value| &value.source))
    {
        if source.digest.is_some() {
            paths.insert(source.path.clone());
        }
    }
    paths
}

fn bounds_error() -> ApplicationError {
    ApplicationError::environment(
        "certification source exceeds supported bounds: 1 MiB lock, 512 KiB per other input, 256 files and 8 MiB total",
    )
}

fn bounded_read(path: &Path, maximum: u64) -> Result<Vec<u8>, ApplicationError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| {
        ApplicationError::environment(
            "certification source input must be an available regular file",
        )
    })?;
    let metadata = file
        .metadata()
        .map_err(|_| ApplicationError::environment("certification source input is unavailable"))?;
    if !metadata.is_file() {
        return Err(ApplicationError::environment(
            "certification source inputs must be regular files",
        ));
    }
    if metadata.len() > maximum {
        return Err(bounds_error());
    }
    let mut bytes = Vec::new();
    file.take(maximum + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            ApplicationError::environment("certification source input could not be read")
        })?;
    if bytes.len() as u64 > maximum {
        return Err(bounds_error());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn directory_provenance_without_a_digest_is_not_read_as_a_file() {
        use ayni_core::{
            Architecture, EnvironmentCertificateTrustPolicy, EnvironmentPlan, Language, Libc,
            OperatingSystem, RepositoryIdentity, RequirementConfidence, RequirementSource,
            RuntimeRequirement, TargetEnvironment, TargetIdentity, TargetPlatform,
            VersionRequirement,
        };
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().canonicalize().expect("repository root");
        let policy = b"[checks]\nsize = true\n";
        fs::write(root.join(".ayni.toml"), policy).expect("policy");
        let target = TargetEnvironment {
            target: TargetIdentity::new(Language::Rust, ".").expect("identity"),
            workspace: None,
            package: None,
            runtimes: vec![RuntimeRequirement {
                runtime: "rust".into(),
                version: VersionRequirement::exact("1.94.0").expect("version"),
                components: vec![],
                targets: vec![],
                source: RequirementSource::new(
                    "catalog",
                    ".",
                    None::<String>,
                    RequirementConfidence::Exact,
                )
                .expect("source"),
            }],
            package_manager: None,
            signal_tools: vec![],
            system_requirements: vec![],
            dependency_locks: vec![],
        };
        let plan = EnvironmentPlan::new(
            RepositoryIdentity {
                name: "static-source-fixture".into(),
                contract_digest: ayni_core::sha256_fingerprint(policy),
            },
            [Architecture::Amd64, Architecture::Arm64]
                .into_iter()
                .map(|architecture| TargetPlatform {
                    os: OperatingSystem::Linux,
                    architecture,
                    libc: Libc::Glibc,
                })
                .collect(),
            vec![target],
            vec![],
            vec![],
        )
        .expect("plan")
        .resolve()
        .expect("resolved plan");
        let base = ayni_environment::resolve_provisioning_base(env!("CARGO_PKG_VERSION"), None)
            .expect("built-in provisioning definition");
        let trust = EnvironmentCertificateTrustPolicy::new(std::collections::BTreeMap::new())
            .expect("local trust");
        let lock = EnvironmentLock::from_resolved_plan(
            &plan,
            env!("CARGO_PKG_VERSION"),
            base.mise_version.clone(),
            base,
            &trust,
            ".ayni.toml",
            &std::collections::BTreeMap::new(),
        )
        .expect("official lock");
        fs::write(
            root.join(LOCK_FILE),
            lock.canonical_json().expect("serialized lock"),
        )
        .expect("lock");
        assert!(root.is_dir());
        assert_eq!(input_paths(&lock), BTreeSet::from([".ayni.toml".into()]));
        assert_eq!(
            validated_lock(&root)
                .expect("directory provenance skipped")
                .fingerprint(),
            lock.fingerprint()
        );
    }

    #[test]
    fn oversized_locks_and_sources_are_rejected_before_lock_interpretation() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join(LOCK_FILE);
        fs::File::create(&path)
            .expect("lock")
            .set_len(LOCK_BYTES + 1)
            .expect("size");
        assert!(
            validated_lock(directory.path())
                .expect_err("oversized lock")
                .message
                .contains("bounds")
        );
        assert!(bounded_read(directory.path(), INPUT_BYTES).is_err());
        let input = directory.path().join("input");
        fs::File::create(&input)
            .expect("input")
            .set_len(INPUT_BYTES + 1)
            .expect("size");
        assert!(bounded_read(&input, INPUT_BYTES).is_err());
        #[cfg(unix)]
        {
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(&input, &link).expect("symlink");
            assert!(bounded_read(&link, INPUT_BYTES).is_err());
        }
    }
}
