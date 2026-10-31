//! Static committed-input validation and explicit staged filesystem certification.
//! This path never discovers adapters, invokes tools or reads signing environment variables.
use crate::application::EnvCertifyOperation;
use crate::application_error::ApplicationError;
use ayni_adapters_common::repository::contained_existing_path;
use ayni_core::{AyniPolicy, EnvironmentLock, sha256_fingerprint};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

mod source;

pub(crate) fn run(operation: EnvCertifyOperation) -> ExitCode {
    match certify(&operation) {
        Ok(report) => {
            println!("{report}");
            ExitCode::SUCCESS
        }
        Err(error) => crate::application_error::render_error(error),
    }
}

fn certify(operation: &EnvCertifyOperation) -> Result<String, ApplicationError> {
    let source = operation
        .repo_root
        .canonicalize()
        .map_err(|_| ApplicationError::input("frozen repository root is unavailable"))?;
    let lock = source::validated_lock(&source)?;
    let trust = committed_trust(&source, &lock)?;
    validate_generic_sources(&source, &lock)?;
    let (root, output) = certification_paths(operation, &source)?;
    let seed = read_seed(&operation.signing_key_file)?;
    let expectations = ayni_environment::staged::CertificationExpectations {
        platform: &operation.platform,
        ayni_executable_digest: &operation.expected_ayni_sha256,
        protected_content_root: &operation.expected_content_root,
    };
    let layer = ayni_environment::staged::certify_filesystem(
        &root,
        &lock,
        &trust,
        &expectations,
        &operation.key_id,
        &seed,
    )
    .map_err(ApplicationError::from)?;
    let report = serde_json::to_string(&layer.report)
        .map_err(|_| ApplicationError::execution("failed to serialize certification report"))?;
    layer.write(&output).map_err(ApplicationError::from)?;
    Ok(report)
}

fn committed_trust(
    source: &Path,
    lock: &EnvironmentLock,
) -> Result<ayni_core::EnvironmentCertificateTrustPolicy, ApplicationError> {
    let policy_path = contained_existing_path(source, Path::new(&lock.repository().contract_path))
        .map_err(|_| {
            ApplicationError::environment("committed policy is outside the frozen repository")
        })?;
    let policy_bytes = fs::read(&policy_path)
        .map_err(|_| ApplicationError::environment("committed policy is unavailable"))?;
    if sha256_fingerprint(&policy_bytes) != lock.repository().contract_digest {
        return Err(ApplicationError::environment(
            "committed policy changed after lock validation",
        ));
    }
    let policy = AyniPolicy::parse(
        std::str::from_utf8(&policy_bytes)
            .map_err(|_| ApplicationError::environment("committed policy is invalid"))?,
    )
    .map_err(|_| ApplicationError::environment("committed policy is invalid"))?;
    policy
        .environment_certificate_trust_policy()
        .map_err(ApplicationError::environment)
}

fn certification_paths(
    operation: &EnvCertifyOperation,
    source: &Path,
) -> Result<(PathBuf, PathBuf), ApplicationError> {
    let root = operation
        .filesystem_root
        .canonicalize()
        .map_err(|_| ApplicationError::input("candidate filesystem root is unavailable"))?;
    if !root.is_dir() {
        return Err(ApplicationError::input(
            "candidate filesystem root must be a directory",
        ));
    }
    let key_path = operation
        .signing_key_file
        .canonicalize()
        .map_err(|_| key_error())?;
    let output = output_path(&operation.output)?;
    if key_path.starts_with(source)
        || output.starts_with(source)
        || !outside_candidate(&root, &key_path)
        || !outside_candidate(&root, &output)
        || key_path == output
    {
        return Err(ApplicationError::input(
            "signing key and output must be separate from each other, outside the frozen repository and outside candidate protected content",
        ));
    }
    Ok((root, output))
}

fn validate_generic_sources(source: &Path, lock: &EnvironmentLock) -> Result<(), ApplicationError> {
    for input in lock
        .tools()
        .iter()
        .map(|value| &value.source)
        .chain(lock.debian_packages().iter().map(|value| &value.source))
    {
        if let Some(expected) = &input.digest {
            let path = contained_existing_path(source, Path::new(&input.path)).map_err(|_| {
                ApplicationError::environment(
                    "locked repository-wide source is unavailable or outside the repository",
                )
            })?;
            if !path.is_file() {
                return Err(ApplicationError::environment(
                    "locked repository-wide source must be a regular file",
                ));
            }
            let bytes = fs::read(path).map_err(|_| {
                ApplicationError::environment("locked repository-wide source is unavailable")
            })?;
            if sha256_fingerprint(bytes) != *expected {
                return Err(ApplicationError::environment(
                    "locked repository-wide source changed; regenerate the committed environment lock",
                ));
            }
        }
    }
    Ok(())
}

fn output_path(path: &Path) -> Result<PathBuf, ApplicationError> {
    let parent = path
        .parent()
        .filter(|value| !value.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent = parent.canonicalize().map_err(|_| {
        ApplicationError::input("metadata layer output directory must already exist")
    })?;
    let name = path
        .file_name()
        .ok_or_else(|| ApplicationError::input("metadata layer output must name a file"))?;
    Ok(parent.join(name))
}

fn outside_candidate(root: &Path, path: &Path) -> bool {
    if root != Path::new("/") {
        return !path.starts_with(root);
    }
    !ayni_environment::PROTECTED_FILE_ROOTS
        .iter()
        .chain(ayni_environment::PROTECTED_TREE_ROOTS)
        .any(|protected| path.starts_with(protected))
        && !path.starts_with("/etc/ayni")
}

fn key_error() -> ApplicationError {
    ApplicationError::input(
        "signing key file must be an available regular, non-symlink file containing exactly 64 lowercase hexadecimal characters and an optional final LF",
    )
}

fn read_seed(path: &Path) -> Result<String, ApplicationError> {
    let file = open_seed_file(path)?;
    let mut bytes = Vec::new();
    file.take(66)
        .read_to_end(&mut bytes)
        .map_err(|_| key_error())?;
    decode_seed(bytes)
}

fn open_seed_file(path: &Path) -> Result<fs::File, ApplicationError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| key_error())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || !(64..=65).contains(&metadata.len())
    {
        return Err(key_error());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| key_error())?;
    let opened = file.metadata().map_err(|_| key_error())?;
    if !opened.is_file() || !(64..=65).contains(&opened.len()) {
        return Err(key_error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.dev() != opened.dev() || metadata.ino() != opened.ino() {
            return Err(key_error());
        }
    }
    Ok(file)
}

fn decode_seed(mut bytes: Vec<u8>) -> Result<String, ApplicationError> {
    if bytes.len() == 65 && bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    if bytes.len() != 64
        || !bytes
            .iter()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(value))
    {
        return Err(key_error());
    }
    String::from_utf8(bytes).map_err(|_| key_error())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signing_file_reads_are_bounded_explicit_and_do_not_echo_rejected_values() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("signing-key");
        for suffix in ["", "\n"] {
            let value = format!("{}{suffix}", "a".repeat(64));
            fs::write(&path, value).expect("write synthetic key");
            assert_eq!(read_seed(&path).expect("key"), "a".repeat(64));
        }
        for value in [
            "F".repeat(64),
            "a".repeat(63),
            "a".repeat(66),
            format!("{}\r\n", "a".repeat(64)),
        ] {
            fs::write(&path, &value).expect("write malformed synthetic key");
            let error = read_seed(&path).expect_err("malformed key rejected");
            assert!(!error.message.contains(&value));
        }
        assert!(read_seed(directory.path()).is_err());
        #[cfg(unix)]
        {
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(&path, &link).expect("key symlink");
            assert!(read_seed(&link).is_err());
        }
    }
    #[test]
    fn secrets_and_outputs_cannot_enter_candidate_protected_content() {
        assert!(outside_candidate(
            Path::new("/"),
            Path::new("/run/secrets/signing-key")
        ));
        assert!(outside_candidate(
            Path::new("/"),
            Path::new("/output/metadata-layer.tar")
        ));
        for path in [
            "/etc/ayni/key",
            "/usr/local/bin/ayni",
            "/opt/ayni/key",
            "/home/ayni/.rustup/key",
        ] {
            assert!(!outside_candidate(Path::new("/"), Path::new(path)));
        }
        assert!(!outside_candidate(
            Path::new("/candidate"),
            Path::new("/candidate/key")
        ));
        assert!(outside_candidate(
            Path::new("/candidate"),
            Path::new("/signer/key")
        ));
    }
}
