use crate::application::CheckOperation;
use crate::application_error::ApplicationError;
use ayni_core::{EnvironmentCertificateEnvelope, sha256_fingerprint};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub(crate) const METADATA_PATH: &str = "/etc/ayni/runtime.json";
const PROTECTED_EXECUTABLE_PATH: &str = "/usr/local/bin/ayni";
static ACTIVE_RUNTIME: OnceLock<RuntimeIdentity> = OnceLock::new();

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(crate) struct RuntimeIdentity {
    pub metadata_path: String,
    pub metadata_digest: String,
    pub certificate: EnvironmentCertificateEnvelope,
}

pub(crate) fn discover() -> Result<Option<RuntimeIdentity>, ApplicationError> {
    let path = Path::new(METADATA_PATH);
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ApplicationError::environment(format!(
                "failed to inspect prebuilt environment metadata {}: {error}",
                path.display()
            )));
        }
    };
    if !running_protected_executable()? {
        return Ok(None);
    }
    validate_metadata_file(path, &metadata)?;
    let bytes = fs::read(path).map_err(|error| {
        ApplicationError::environment(format!(
            "failed to read prebuilt environment metadata {}: {error}",
            path.display()
        ))
    })?;
    let certificate =
        serde_json::from_slice::<EnvironmentCertificateEnvelope>(&bytes).map_err(|error| {
            ApplicationError::environment(format!(
                "malformed prebuilt environment metadata {}: {error}",
                path.display()
            ))
        })?;
    certificate
        .certificate
        .canonical_payload()
        .map_err(|error| {
            ApplicationError::environment(format!(
                "malformed prebuilt environment metadata {}: {error}",
                path.display()
            ))
        })?;
    if certificate.key_id.trim().is_empty()
        || certificate.signature.len() != 128
        || !certificate
            .signature
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(ApplicationError::environment(format!(
            "malformed prebuilt environment metadata {}: invalid certificate envelope",
            path.display()
        )));
    }
    Ok(Some(RuntimeIdentity {
        metadata_path: METADATA_PATH.into(),
        metadata_digest: sha256_fingerprint(&bytes),
        certificate,
    }))
}

pub(crate) fn prepare_check(
    mut operation: CheckOperation,
    runtime: &RuntimeIdentity,
    registry: &ayni_core::AdapterRegistry,
) -> Result<CheckOperation, ApplicationError> {
    let root = source_root()?;
    let config = root.join(".ayni.toml");
    let lock = root.join(".ayni.lock");
    for path in [&config, &lock] {
        let metadata = fs::symlink_metadata(path).map_err(|error| {
            ApplicationError::input(format!(
                "prebuilt environment source requires {}: {error}",
                path.display()
            ))
        })?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(ApplicationError::input(format!(
                "prebuilt environment source requires {} to be a regular file",
                path.display()
            )));
        }
    }
    let source_lock =
        crate::environment_backend::validate_prebuilt_source(&root, &config, registry)
            .map_err(ApplicationError::from)?;
    if source_lock != runtime.certificate.certificate.lock_fingerprint {
        return Err(ApplicationError::environment(format!(
            "prebuilt environment metadata lock fingerprint {} does not match mounted source lock {}; mount the source used to build this environment",
            runtime.certificate.certificate.lock_fingerprint, source_lock
        )));
    }
    operation.config = config;
    Ok(operation)
}

pub(crate) fn activate(runtime: &RuntimeIdentity) -> Result<(), ApplicationError> {
    ACTIVE_RUNTIME.set(runtime.clone()).map_err(|_| {
        ApplicationError::execution("prebuilt environment runtime was activated more than once")
    })
}

pub(crate) fn active_identity() -> Option<ayni_core::PrebuiltRuntimeIdentity> {
    ACTIVE_RUNTIME
        .get()
        .map(|runtime| ayni_core::PrebuiltRuntimeIdentity {
            metadata_path: runtime.metadata_path.clone(),
            metadata_digest: runtime.metadata_digest.clone(),
            certificate: runtime.certificate.clone(),
        })
}

pub(crate) fn active() -> bool {
    ACTIVE_RUNTIME.get().is_some()
}

fn running_protected_executable() -> Result<bool, ApplicationError> {
    let current = std::env::current_exe().map_err(|error| {
        ApplicationError::environment(format!(
            "failed to identify the running Ayni executable: {error}"
        ))
    })?;
    let protected = match fs::canonicalize(PROTECTED_EXECUTABLE_PATH) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(ApplicationError::environment(format!(
                "failed to resolve protected Ayni executable {PROTECTED_EXECUTABLE_PATH}: {error}"
            )));
        }
    };
    Ok(current == protected)
}

fn source_root() -> Result<PathBuf, ApplicationError> {
    let configured = std::env::var_os("AYNI_SOURCE_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let root = configured.canonicalize().map_err(|error| {
        ApplicationError::input(format!(
            "failed to resolve prebuilt environment source root {}: {error}",
            configured.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(ApplicationError::input(format!(
            "prebuilt environment source root is not a directory: {}",
            root.display()
        )));
    }
    Ok(root)
}

#[cfg(unix)]
fn validate_metadata_file(path: &Path, metadata: &fs::Metadata) -> Result<(), ApplicationError> {
    use std::os::unix::fs::MetadataExt;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.uid() != 0 {
        return Err(ApplicationError::environment(format!(
            "prebuilt environment metadata {} must be a root-owned regular file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_metadata_file(path: &Path, metadata: &fs::Metadata) -> Result<(), ApplicationError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ApplicationError::environment(format!(
            "prebuilt environment metadata {} must be a regular file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::EnvironmentCertificate;

    #[test]
    fn test_binary_does_not_claim_the_protected_runtime_identity() {
        assert!(!running_protected_executable().unwrap());
    }

    #[test]
    fn runtime_identity_serializes_certificate_claims() {
        let certificate = EnvironmentCertificate::new(
            format!("sha256:{}", "a".repeat(64)),
            "linux/amd64",
            "0.14.0",
            format!("sha256:{}", "b".repeat(64)),
            format!("sha256:{}", "c".repeat(64)),
        )
        .unwrap();
        let identity = RuntimeIdentity {
            metadata_path: METADATA_PATH.into(),
            metadata_digest: format!("sha256:{}", "d".repeat(64)),
            certificate: EnvironmentCertificateEnvelope {
                certificate,
                key_id: "release-2026".into(),
                signature: "0".repeat(128),
            },
        };
        let encoded = serde_json::to_string(&identity).unwrap();
        assert!(encoded.contains("lock_fingerprint"));
        assert!(encoded.contains(METADATA_PATH));
    }

    #[cfg(unix)]
    #[test]
    fn metadata_requires_root_ownership() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let error = validate_metadata_file(file.path(), &fs::metadata(file.path()).unwrap())
            .expect_err("fixture is not root owned");
        assert!(error.message.contains("root-owned regular file"));
    }
}
