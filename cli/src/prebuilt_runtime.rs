use crate::application::{CheckOperation, EnvShowOperation, OutputFormat};
use crate::application_error::ApplicationError;
use ayni_core::{EnvironmentCertificateEnvelope, lower_hex, sha256_fingerprint};
use ayni_environment::ProtectedManifestEntry;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
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
    #[serde(skip)]
    pub tool_versions: Vec<ayni_core::ArtifactToolVersion>,
    #[serde(skip)]
    pub target_environments: BTreeMap<String, BTreeMap<String, String>>,
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
    if metadata.len() > ayni_environment::MAX_CERTIFICATE_BYTES {
        return Err(ApplicationError::environment(format!(
            "prebuilt environment metadata {} exceeds the supported size limit",
            path.display()
        )));
    }
    let bytes = fs::read(path).map_err(|error| {
        ApplicationError::environment(format!(
            "failed to read prebuilt environment metadata {}: {error}",
            path.display()
        ))
    })?;
    let certificate = decode_certificate(&bytes, path)?;
    Ok(Some(RuntimeIdentity {
        metadata_path: METADATA_PATH.into(),
        metadata_digest: sha256_fingerprint(&bytes),
        certificate,
        tool_versions: Vec::new(),
        target_environments: BTreeMap::new(),
    }))
}

fn decode_certificate(
    bytes: &[u8],
    path: &Path,
) -> Result<EnvironmentCertificateEnvelope, ApplicationError> {
    let certificate =
        serde_json::from_slice::<EnvironmentCertificateEnvelope>(bytes).map_err(|error| {
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
        || certificate.key_id.len() > ayni_environment::MAX_CERTIFICATE_KEY_ID_BYTES
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
    let mut canonical = serde_json::to_vec(&certificate).map_err(|error| {
        ApplicationError::environment(format!(
            "failed to canonicalize prebuilt environment metadata {}: {error}",
            path.display()
        ))
    })?;
    canonical.push(b'\n');
    if canonical != bytes {
        return Err(ApplicationError::environment(format!(
            "prebuilt environment metadata {} is not canonically encoded",
            path.display()
        )));
    }
    Ok(certificate)
}

struct VerifiedRuntimeContext {
    tool_versions: Vec<ayni_core::ArtifactToolVersion>,
    target_environments: BTreeMap<String, BTreeMap<String, String>>,
}

pub(crate) fn prepare_check(
    mut operation: CheckOperation,
    runtime: &mut RuntimeIdentity,
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
    crate::environment_backend::validate_prebuilt_source(&root, &config, registry)
        .map_err(ApplicationError::from)?;
    let verified = verify_runtime(runtime, &root, &config)?;
    runtime.tool_versions = verified.tool_versions;
    runtime.target_environments = verified.target_environments;
    operation.config = config;
    Ok(operation)
}

fn verify_runtime(
    runtime: &RuntimeIdentity,
    root: &Path,
    config: &Path,
) -> Result<VerifiedRuntimeContext, ApplicationError> {
    let lock = ayni_environment::read_lock(root).map_err(ApplicationError::from)?;
    let context = EnvShowOperation {
        config: config.to_path_buf(),
        repo_root: root.to_path_buf(),
        output: OutputFormat::Json,
    };
    let (_, _, _, policy) = crate::environment::load_context(&context)?;
    let trust = policy
        .environment_certificate_trust_policy()
        .map_err(ApplicationError::environment)?;
    verify_certificate(&runtime.certificate, &trust)?;

    let plan = ayni_environment::image_plan(&lock).map_err(ApplicationError::from)?;
    verify_claims(
        &runtime.certificate.certificate,
        lock.fingerprint(),
        &plan.platform,
        env!("CARGO_PKG_VERSION"),
        &plan.installation_digest,
    )?;
    let claims = &runtime.certificate.certificate;

    let manifest_path = Path::new("/etc/ayni/protected-content.manifest");
    validate_protected_file(manifest_path, "protected-content manifest")?;
    let manifest_metadata = fs::metadata(manifest_path).map_err(|error| {
        ApplicationError::environment(format!(
            "failed to inspect protected-content manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    if manifest_metadata.len() > ayni_environment::MAX_PROTECTED_MANIFEST_BYTES {
        return Err(ApplicationError::environment(
            "protected-content manifest exceeds the supported size limit",
        ));
    }
    let manifest = fs::read_to_string(manifest_path).map_err(|error| {
        ApplicationError::environment(format!(
            "failed to read protected-content manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    if sha256_fingerprint(manifest.as_bytes()) != claims.protected_content_root {
        return Err(ApplicationError::environment(
            "prebuilt environment certificate protected-content root does not match its manifest",
        ));
    }
    verify_manifest_at(&manifest, Path::new("/"))?;
    Ok(VerifiedRuntimeContext {
        tool_versions: ayni_environment::locked_tool_versions(&lock),
        target_environments: ayni_environment::locked_target_environments(&lock)
            .map_err(ApplicationError::from)?,
    })
}

fn verify_certificate(
    certificate: &EnvironmentCertificateEnvelope,
    trust: &ayni_core::EnvironmentCertificateTrustPolicy,
) -> Result<(), ApplicationError> {
    certificate.verify(trust).map_err(|error| {
        ApplicationError::environment(format!(
            "prebuilt environment certificate verification failed: {error}"
        ))
    })
}

fn verify_claims(
    claims: &ayni_core::EnvironmentCertificate,
    lock_fingerprint: &str,
    platform: &str,
    ayni_version: &str,
    tool_inventory_digest: &str,
) -> Result<(), ApplicationError> {
    if claims.lock_fingerprint != lock_fingerprint
        || claims.platform != platform
        || claims.ayni_version != ayni_version
        || claims.tool_inventory_digest != tool_inventory_digest
    {
        return Err(ApplicationError::environment(
            "prebuilt environment certificate claims do not match the mounted source lock or runner",
        ));
    }
    Ok(())
}

fn verify_manifest_at(manifest: &str, filesystem_root: &Path) -> Result<(), ApplicationError> {
    let expected =
        ayni_environment::parse_protected_manifest(manifest).map_err(ApplicationError::from)?;
    let actual = protected_inventory(filesystem_root)?;
    compare_inventory(&expected, &actual)
}

fn manifest_path(filesystem_root: &Path, path: &str) -> PathBuf {
    filesystem_root.join(path.trim_start_matches('/'))
}

fn protected_inventory(
    filesystem_root: &Path,
) -> Result<Vec<ProtectedManifestEntry>, ApplicationError> {
    let enforce_protection = filesystem_root == Path::new("/");
    let mut entries = Vec::new();
    for logical in ayni_environment::PROTECTED_FILE_ROOTS {
        validate_protected_ancestors(filesystem_root, logical, enforce_protection)?;
        inventory_path(
            filesystem_root,
            logical,
            None,
            enforce_protection,
            &mut entries,
        )?;
    }
    for logical in ayni_environment::PROTECTED_TREE_ROOTS {
        let actual = manifest_path(filesystem_root, logical);
        match fs::symlink_metadata(&actual) {
            Ok(metadata) => {
                validate_protected_ancestors(filesystem_root, logical, enforce_protection)?;
                #[cfg(unix)]
                let device = {
                    use std::os::unix::fs::MetadataExt;
                    Some(metadata.dev())
                };
                #[cfg(not(unix))]
                let device = None;
                inventory_path(
                    filesystem_root,
                    logical,
                    device,
                    enforce_protection,
                    &mut entries,
                )?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(ApplicationError::environment(format!(
                    "failed to inspect protected-content root {logical}: {error}"
                )));
            }
        }
    }
    if entries.len() > ayni_environment::MAX_PROTECTED_MANIFEST_ENTRIES {
        return Err(ApplicationError::environment(
            "protected-content inventory exceeds the supported entry limit",
        ));
    }
    entries.sort_by(|left, right| left.path().cmp(right.path()).then(left.cmp(right)));
    Ok(entries)
}

fn validate_protected_ancestors(
    filesystem_root: &Path,
    logical: &str,
    enforce_protection: bool,
) -> Result<(), ApplicationError> {
    if !enforce_protection {
        return Ok(());
    }
    for ancestor in ayni_environment::PROTECTED_ANCESTOR_DIRS
        .iter()
        .filter(|ancestor| **ancestor == "/" || logical.starts_with(&format!("{ancestor}/")))
    {
        let actual = manifest_path(filesystem_root, ancestor);
        let metadata = fs::symlink_metadata(&actual).map_err(|error| {
            ApplicationError::environment(format!(
                "failed to inspect protected-content ancestor {ancestor}: {error}"
            ))
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(ApplicationError::environment(format!(
                "protected-content ancestor {ancestor} must be a directory"
            )));
        }
        validate_inventory_metadata(ancestor, &metadata, None, true)?;
    }
    Ok(())
}

fn inventory_path(
    filesystem_root: &Path,
    logical: &str,
    expected_device: Option<u64>,
    enforce_protection: bool,
    entries: &mut Vec<ProtectedManifestEntry>,
) -> Result<(), ApplicationError> {
    let actual = manifest_path(filesystem_root, logical);
    let metadata = fs::symlink_metadata(&actual).map_err(|error| {
        ApplicationError::environment(format!(
            "protected-content path {logical} is unavailable: {error}"
        ))
    })?;
    validate_inventory_metadata(logical, &metadata, expected_device, enforce_protection)?;
    if metadata.is_file() && !metadata.file_type().is_symlink() {
        entries.push(ProtectedManifestEntry::File {
            path: logical.into(),
            sha256: hash_protected_file(&actual, logical, &metadata)?,
        });
    } else if metadata.file_type().is_symlink() {
        let target = fs::read_link(&actual).map_err(|error| {
            ApplicationError::environment(format!(
                "failed to read protected-content symlink {logical}: {error}"
            ))
        })?;
        let target = target.to_str().ok_or_else(|| {
            ApplicationError::environment(format!(
                "protected-content symlink {logical} has a non-UTF-8 target"
            ))
        })?;
        entries.push(ProtectedManifestEntry::Symlink {
            path: logical.into(),
            target: target.into(),
        });
    } else if metadata.is_dir() {
        inventory_directory(
            filesystem_root,
            logical,
            &actual,
            expected_device,
            enforce_protection,
            entries,
        )?;
    } else {
        return Err(ApplicationError::environment(format!(
            "protected-content path {logical} has an unsupported file type"
        )));
    }
    Ok(())
}

fn inventory_directory(
    filesystem_root: &Path,
    logical: &str,
    actual: &Path,
    expected_device: Option<u64>,
    enforce_protection: bool,
    entries: &mut Vec<ProtectedManifestEntry>,
) -> Result<(), ApplicationError> {
    let mut children = fs::read_dir(actual)
        .map_err(|error| {
            ApplicationError::environment(format!(
                "failed to read protected-content directory {logical}: {error}"
            ))
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            ApplicationError::environment(format!(
                "failed to enumerate protected-content directory {logical}: {error}"
            ))
        })?;
    children.sort_by_key(|entry| entry.file_name());
    for child in children {
        let name = child.file_name().into_string().map_err(|_| {
            ApplicationError::environment(format!(
                "protected-content directory {logical} contains a non-UTF-8 name"
            ))
        })?;
        inventory_path(
            filesystem_root,
            &format!("{logical}/{name}"),
            expected_device,
            enforce_protection,
            entries,
        )?;
    }
    Ok(())
}

fn hash_protected_file(
    path: &Path,
    logical: &str,
    expected_metadata: &fs::Metadata,
) -> Result<String, ApplicationError> {
    let mut file = fs::File::open(path).map_err(|error| {
        ApplicationError::environment(format!(
            "failed to open protected-content file {logical}: {error}"
        ))
    })?;
    let opened = file.metadata().map_err(|error| {
        ApplicationError::environment(format!(
            "failed to inspect opened protected-content file {logical}: {error}"
        ))
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != expected_metadata.dev() || opened.ino() != expected_metadata.ino() {
            return Err(ApplicationError::environment(format!(
                "protected-content file {logical} changed while it was inspected"
            )));
        }
    }
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            ApplicationError::environment(format!(
                "failed to read protected-content file {logical}: {error}"
            ))
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("sha256:{}", lower_hex(hasher.finalize())))
}

fn validate_inventory_metadata(
    logical: &str,
    metadata: &fs::Metadata,
    expected_device: Option<u64>,
    enforce_protection: bool,
) -> Result<(), ApplicationError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if expected_device.is_some_and(|device| metadata.dev() != device) {
            return Err(ApplicationError::environment(format!(
                "protected-content path {logical} crosses a filesystem boundary"
            )));
        }
        if enforce_protection
            && (metadata.uid() != 0
                || metadata.gid() != 0
                || (!metadata.file_type().is_symlink() && metadata.mode() & (0o7000 | 0o022) != 0))
        {
            return Err(ApplicationError::environment(format!(
                "protected-content path {logical} must be root-owned and not writable by the runtime user, with no special mode bits"
            )));
        }
    }
    let _ = (expected_device, enforce_protection);
    Ok(())
}

fn compare_inventory(
    expected: &[ProtectedManifestEntry],
    actual: &[ProtectedManifestEntry],
) -> Result<(), ApplicationError> {
    let mut expected_index = 0;
    let mut actual_index = 0;
    while expected_index < expected.len() || actual_index < actual.len() {
        match (expected.get(expected_index), actual.get(actual_index)) {
            (Some(expected), Some(actual)) if expected.path() == actual.path() => {
                if expected != actual {
                    return Err(inventory_mismatch(expected, actual));
                }
                expected_index += 1;
                actual_index += 1;
            }
            (Some(expected), Some(actual)) if expected.path() < actual.path() => {
                return Err(ApplicationError::environment(format!(
                    "protected-content path {} declared by the manifest is unavailable",
                    expected.path()
                )));
            }
            (Some(_), Some(actual)) => {
                return Err(ApplicationError::environment(format!(
                    "protected-content inventory contains unlisted path {}",
                    actual.path()
                )));
            }
            (Some(expected), None) => {
                return Err(ApplicationError::environment(format!(
                    "protected-content path {} declared by the manifest is unavailable",
                    expected.path()
                )));
            }
            (None, Some(actual)) => {
                return Err(ApplicationError::environment(format!(
                    "protected-content inventory contains unlisted path {}",
                    actual.path()
                )));
            }
            (None, None) => break,
        }
    }
    Ok(())
}

fn inventory_mismatch(
    expected: &ProtectedManifestEntry,
    actual: &ProtectedManifestEntry,
) -> ApplicationError {
    match (expected, actual) {
        (
            ProtectedManifestEntry::File { path, sha256 },
            ProtectedManifestEntry::File { sha256: actual, .. },
        ) if sha256 != actual => ApplicationError::environment(format!(
            "protected-content file {path} does not match its manifest digest"
        )),
        (
            ProtectedManifestEntry::Symlink { path, target },
            ProtectedManifestEntry::Symlink { target: actual, .. },
        ) if target != actual => ApplicationError::environment(format!(
            "protected-content symlink {path} does not match its manifest target"
        )),
        _ => ApplicationError::environment(format!(
            "protected-content path {} has a different type than its manifest entry",
            expected.path()
        )),
    }
}

fn validate_protected_file(path: &Path, description: &str) -> Result<(), ApplicationError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        ApplicationError::environment(format!(
            "failed to inspect {description} {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(ApplicationError::environment(format!(
            "{description} {} must be a regular file",
            path.display()
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != 0 || metadata.gid() != 0 || metadata.mode() & 0o7777 != 0o444 {
            return Err(ApplicationError::environment(format!(
                "{description} {} must be root-owned and mode 0444",
                path.display()
            )));
        }
    }
    Ok(())
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

pub(crate) fn active_tool_versions() -> Vec<ayni_core::ArtifactToolVersion> {
    ACTIVE_RUNTIME
        .get()
        .map_or_else(Vec::new, |runtime| runtime.tool_versions.clone())
}

pub(crate) fn active_target_environment(
    language: ayni_core::Language,
    root: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(runtime) = ACTIVE_RUNTIME.get() else {
        return Ok(None);
    };
    let key = format!("{language}:{root}");
    runtime
        .target_environments
        .get(&key)
        .cloned()
        .map(Some)
        .ok_or_else(|| format!("prebuilt environment has no locked activation for target {key}"))
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
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != 0
        || metadata.gid() != 0
        || metadata.mode() & 0o7777 != 0o444
    {
        return Err(ApplicationError::environment(format!(
            "prebuilt environment metadata {} must be a root-owned mode 0444 regular file",
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
    use ayni_core::{EnvironmentCertificate, EnvironmentCertificateTrustPolicy};
    use ed25519_dalek::SigningKey;
    use std::collections::BTreeMap;

    fn digest(byte: char) -> String {
        format!("sha256:{}", byte.to_string().repeat(64))
    }

    fn certificate() -> EnvironmentCertificate {
        EnvironmentCertificate::new(
            digest('a'),
            "linux/amd64",
            env!("CARGO_PKG_VERSION"),
            digest('b'),
            digest('c'),
        )
        .unwrap()
    }

    fn signed_certificate() -> (
        EnvironmentCertificateEnvelope,
        EnvironmentCertificateTrustPolicy,
    ) {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let envelope =
            EnvironmentCertificateEnvelope::sign(certificate(), "release-2026", &signing_key)
                .unwrap();
        let trust = EnvironmentCertificateTrustPolicy::new(BTreeMap::from([(
            String::from("release-2026"),
            ayni_core::lower_hex(signing_key.verifying_key().to_bytes()),
        )]))
        .unwrap();
        (envelope, trust)
    }

    fn write_file(root: &Path, path: &str, bytes: &[u8]) {
        let path = manifest_path(root, path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }

    #[cfg(unix)]
    fn protected_manifest(root: &Path) -> String {
        use std::os::unix::fs::symlink;

        let files = [
            ("/etc/ayni/mise.toml", b"[tools]\n".as_slice()),
            ("/usr/local/bin/ayni", b"ayni".as_slice()),
            ("/usr/local/bin/mise", b"mise".as_slice()),
            (
                "/opt/ayni/mise/installs/rust/bin/rustc",
                b"rustc".as_slice(),
            ),
        ];
        let mut entries = files
            .into_iter()
            .map(|(path, bytes)| {
                write_file(root, path, bytes);
                ProtectedManifestEntry::File {
                    path: path.into(),
                    sha256: sha256_fingerprint(bytes),
                }
            })
            .collect::<Vec<_>>();
        let link = "/opt/ayni/mise/shims/rustc";
        let target = "../installs/rust/bin/rustc";
        let link_path = manifest_path(root, link);
        fs::create_dir_all(link_path.parent().unwrap()).unwrap();
        symlink(target, link_path).unwrap();
        entries.push(ProtectedManifestEntry::Symlink {
            path: link.into(),
            target: target.into(),
        });
        entries.sort_by(|left, right| left.path().cmp(right.path()).then(left.cmp(right)));
        let mut manifest = String::from("{\"schema_version\":\"1\"}\n");
        for entry in entries {
            manifest.push_str(&serde_json::to_string(&entry).unwrap());
            manifest.push('\n');
        }
        manifest
    }

    #[test]
    fn test_binary_does_not_claim_the_protected_runtime_identity() {
        assert!(!running_protected_executable().unwrap());
    }

    #[test]
    fn runtime_identity_serializes_certificate_claims() {
        let identity = RuntimeIdentity {
            metadata_path: METADATA_PATH.into(),
            metadata_digest: digest('d'),
            certificate: signed_certificate().0,
            tool_versions: Vec::new(),
            target_environments: BTreeMap::new(),
        };
        let encoded = serde_json::to_string(&identity).unwrap();
        assert!(encoded.contains("lock_fingerprint"));
        assert!(encoded.contains(METADATA_PATH));
    }

    #[test]
    fn certificate_metadata_requires_canonical_json_with_one_newline() {
        let envelope = signed_certificate().0;
        let mut canonical = serde_json::to_vec(&envelope).unwrap();
        canonical.push(b'\n');
        assert_eq!(
            decode_certificate(&canonical, Path::new(METADATA_PATH)).unwrap(),
            envelope
        );

        let pretty = serde_json::to_vec_pretty(&envelope).unwrap();
        let error = decode_certificate(&pretty, Path::new(METADATA_PATH)).unwrap_err();
        assert!(error.message.contains("not canonically encoded"));
        canonical.push(b'\n');
        let error = decode_certificate(&canonical, Path::new(METADATA_PATH)).unwrap_err();
        assert!(error.message.contains("not canonically encoded"));
    }

    #[test]
    fn certificate_verification_rejects_invalid_signatures_and_untrusted_keys() {
        let (envelope, trust) = signed_certificate();
        verify_certificate(&envelope, &trust).unwrap();

        let mut invalid = envelope.clone();
        invalid.signature.replace_range(..2, "00");
        let error = verify_certificate(&invalid, &trust).unwrap_err();
        assert!(error.message.contains("signature is invalid"));

        let mut untrusted = envelope;
        untrusted.key_id = String::from("other");
        let error = verify_certificate(&untrusted, &trust).unwrap_err();
        assert!(error.message.contains("is not trusted"));
    }

    #[test]
    fn certificate_claims_bind_lock_platform_version_and_tools() {
        let claims = certificate();
        verify_claims(
            &claims,
            &digest('a'),
            "linux/amd64",
            env!("CARGO_PKG_VERSION"),
            &digest('b'),
        )
        .unwrap();
        for expected in [
            (
                digest('d'),
                String::from("linux/amd64"),
                String::from(env!("CARGO_PKG_VERSION")),
                digest('b'),
            ),
            (
                digest('a'),
                String::from("linux/arm64"),
                String::from(env!("CARGO_PKG_VERSION")),
                digest('b'),
            ),
            (
                digest('a'),
                String::from("linux/amd64"),
                String::from("999.0.0"),
                digest('b'),
            ),
            (
                digest('a'),
                String::from("linux/amd64"),
                String::from(env!("CARGO_PKG_VERSION")),
                digest('d'),
            ),
        ] {
            assert!(
                verify_claims(&claims, &expected.0, &expected.1, &expected.2, &expected.3)
                    .unwrap_err()
                    .message
                    .contains("claims do not match")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn manifest_verifies_files_and_exact_symlink_targets() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let manifest = protected_manifest(root.path());
        verify_manifest_at(&manifest, root.path()).unwrap();

        write_file(root.path(), "/usr/local/bin/mise", b"tampered");
        let error = verify_manifest_at(&manifest, root.path()).unwrap_err();
        assert!(error.message.contains("does not match its manifest digest"));
        write_file(root.path(), "/usr/local/bin/mise", b"mise");

        let link = manifest_path(root.path(), "/opt/ayni/mise/shims/rustc");
        fs::remove_file(&link).unwrap();
        symlink("../installs/rust/bin/other", link).unwrap();
        let error = verify_manifest_at(&manifest, root.path()).unwrap_err();
        assert!(error.message.contains("does not match its manifest target"));
    }

    #[cfg(unix)]
    #[test]
    fn manifest_rejects_unlisted_protected_content() {
        let root = tempfile::tempdir().unwrap();
        let manifest = protected_manifest(root.path());
        write_file(
            root.path(),
            "/opt/ayni/mise/installs/rust/bin/unlisted",
            b"unlisted",
        );
        let error = verify_manifest_at(&manifest, root.path()).unwrap_err();
        assert!(error.message.contains("unlisted path"));
    }

    #[test]
    fn manifest_rejects_malformed_noncanonical_and_unsafe_entries() {
        let root = tempfile::tempdir().unwrap();
        for manifest in [
            "{}\n",
            "{\"schema_version\":\"1\"}\nnot-json\n",
            "{\"schema_version\":\"1\"}\n{\"type\":\"file\",\"path\":\"/../escape\",\"sha256\":\"sha256:00\"}\n",
            "{\"schema_version\":\"1\"}\n{ \"type\": \"file\", \"path\": \"/usr/local/bin/ayni\", \"sha256\": \"sha256:00\" }\n",
        ] {
            assert!(
                verify_manifest_at(manifest, root.path()).is_err(),
                "{manifest}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn metadata_requires_root_ownership_and_read_only_mode() {
        use std::os::unix::fs::PermissionsExt;

        let file = tempfile::NamedTempFile::new().unwrap();
        fs::set_permissions(file.path(), fs::Permissions::from_mode(0o600)).unwrap();
        let error = validate_metadata_file(file.path(), &fs::metadata(file.path()).unwrap())
            .expect_err("fixture is not protected");
        assert!(error.message.contains("root-owned mode 0444 regular file"));
    }
}
