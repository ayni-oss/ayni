//! Deterministic in-image environment certification material.
use crate::BackendError;
use crate::image::ImagePlan;
use ayni_core::{
    ENVIRONMENT_CERTIFICATE_SCHEMA_VERSION, EnvironmentCertificate, EnvironmentCertificateEnvelope,
    EnvironmentLock, sha256_fingerprint,
};
use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};
use std::env;

pub(crate) const CERTIFICATE_PATH: &str = "/etc/ayni/runtime.json";
pub(crate) const MANIFEST_PATH: &str = "/etc/ayni/protected-content.manifest";
pub(crate) const CERTIFICATE_SCHEMA_LABEL: &str = "dev.ayni.environment.certificate-schema";
pub(crate) const CERTIFICATE_KEY_ID_LABEL: &str = "dev.ayni.environment.certificate-key-id";
pub(crate) const PROTECTED_CONTENT_ROOT_LABEL: &str = "dev.ayni.environment.protected-content-root";
pub(crate) const MANIFEST_SCHEMA_VERSION: &str = "1";
const SIGNING_KEY_ENV: &str = "AYNI_ENV_CERTIFICATE_SIGNING_KEY";
const KEY_ID_ENV: &str = "AYNI_ENV_CERTIFICATE_KEY_ID";
pub(crate) const SIGNING_ENVIRONMENT: &[&str] = &[SIGNING_KEY_ENV, KEY_ID_ENV];

pub(crate) struct SigningMaterial {
    key_id: String,
    key: SigningKey,
}

impl SigningMaterial {
    pub(crate) fn from_env() -> Result<Self, BackendError> {
        Self::from_values(&required(SIGNING_KEY_ENV)?, &required(KEY_ID_ENV)?)
    }

    fn from_values(seed: &str, key_id: &str) -> Result<Self, BackendError> {
        validate_key_id(key_id)?;
        Ok(Self {
            key_id: key_id.into(),
            key: signing_key(seed)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CertificateIdentity {
    pub(crate) schema_version: String,
    pub(crate) key_id: String,
    pub(crate) protected_content_root: String,
    pub(crate) certificate_digest: String,
}

impl CertificateIdentity {
    pub(crate) fn validate(&self) -> Result<(), BackendError> {
        if self.schema_version != ENVIRONMENT_CERTIFICATE_SCHEMA_VERSION {
            return Err(BackendError::environment(
                "execution build record has an unsupported certificate schema",
            ));
        }
        validate_key_id(&self.key_id)?;
        for (description, digest) in [
            ("protected content root", &self.protected_content_root),
            ("certificate digest", &self.certificate_digest),
        ] {
            if !valid_digest(digest) {
                return Err(BackendError::environment(format!(
                    "execution build record has an invalid {description}"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Certification {
    pub(crate) certificate: String,
    pub(crate) manifest: String,
    pub(crate) identity: CertificateIdentity,
}

pub(crate) fn create(
    lock: &EnvironmentLock,
    plan: &ImagePlan,
    manifest: String,
    signing: &SigningMaterial,
) -> Result<Certification, BackendError> {
    validate_manifest(&manifest)?;
    let protected_content_root = sha256_fingerprint(manifest.as_bytes());
    let certificate = EnvironmentCertificate::new(
        lock.fingerprint(),
        &plan.platform,
        env!("CARGO_PKG_VERSION"),
        &plan.installation_digest,
        &protected_content_root,
    )
    .map_err(|error| {
        BackendError::environment(format!("cannot create environment certificate: {error}"))
    })?;
    let envelope = EnvironmentCertificateEnvelope::sign(certificate, &signing.key_id, &signing.key)
        .map_err(|error| {
            BackendError::environment(format!("cannot sign environment certificate: {error}"))
        })?;
    let mut certificate = serde_json::to_string(&envelope).map_err(|error| {
        BackendError::execution(format!("cannot serialize environment certificate: {error}"))
    })?;
    certificate.push('\n');
    let identity = CertificateIdentity {
        schema_version: ENVIRONMENT_CERTIFICATE_SCHEMA_VERSION.into(),
        key_id: signing.key_id.clone(),
        protected_content_root,
        certificate_digest: sha256_fingerprint(certificate.as_bytes()),
    };
    Ok(Certification {
        certificate,
        manifest,
        identity,
    })
}

pub(crate) fn validate_installed(
    certificate: &str,
    manifest: &str,
    lock: &EnvironmentLock,
    plan: &ImagePlan,
    expected: &CertificateIdentity,
) -> Result<(), BackendError> {
    expected.validate()?;
    validate_manifest(manifest)?;
    let envelope: EnvironmentCertificateEnvelope =
        serde_json::from_str(certificate).map_err(|error| {
            BackendError::environment(format!(
                "installed environment certificate is malformed: {error}"
            ))
        })?;
    envelope.certificate.canonical_payload().map_err(|error| {
        BackendError::environment(format!(
            "installed environment certificate is invalid: {error}"
        ))
    })?;
    let claims = &envelope.certificate;
    let content_root = sha256_fingerprint(manifest.as_bytes());
    let certificate_digest = sha256_fingerprint(certificate.as_bytes());
    if claims.lock_fingerprint != lock.fingerprint()
        || claims.platform != plan.platform
        || claims.ayni_version != env!("CARGO_PKG_VERSION")
        || claims.tool_inventory_digest != plan.installation_digest
        || claims.protected_content_root != content_root
        || claims.schema_version != expected.schema_version
        || envelope.key_id != expected.key_id
        || content_root != expected.protected_content_root
        || certificate_digest != expected.certificate_digest
    {
        return Err(BackendError::environment(
            "installed environment certificate, manifest, and expected build identity differ",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
enum ManifestEntry {
    File { path: String, sha256: String },
    Symlink { path: String, target: String },
}

impl ManifestEntry {
    fn path(&self) -> &str {
        match self {
            Self::File { path, .. } | Self::Symlink { path, .. } => path,
        }
    }
}

#[derive(Serialize)]
struct ManifestHeader<'a> {
    schema_version: &'a str,
}

/// Convert NUL-delimited container inventory output into canonical JSON Lines.
///
/// File records use GNU `sha256sum --zero` (`<hex>  <path>\0`). Symlink records
/// are alternating path/target strings emitted by GNU `find -printf`.
pub(crate) fn manifest_from_inventory(
    file_records: &[u8],
    symlink_records: &[u8],
) -> Result<String, BackendError> {
    let mut entries = parse_file_records(file_records)?;
    entries.extend(parse_symlink_records(symlink_records)?);
    entries.sort_by(compare_manifest_entries);
    validate_manifest_entries(&entries)?;
    Ok(render_manifest(&entries))
}

fn render_manifest(entries: &[ManifestEntry]) -> String {
    let mut manifest = serde_json::to_string(&ManifestHeader {
        schema_version: MANIFEST_SCHEMA_VERSION,
    })
    .expect("manifest header serializes");
    manifest.push('\n');
    for entry in entries {
        manifest.push_str(&serde_json::to_string(entry).expect("manifest entry serializes"));
        manifest.push('\n');
    }
    manifest
}

fn compare_manifest_entries(left: &ManifestEntry, right: &ManifestEntry) -> std::cmp::Ordering {
    left.path().cmp(right.path()).then(left.cmp(right))
}

fn validate_manifest_entries(entries: &[ManifestEntry]) -> Result<(), BackendError> {
    if entries.windows(2).any(|pair| {
        compare_manifest_entries(&pair[0], &pair[1]) != std::cmp::Ordering::Less
            || pair[0].path() == pair[1].path()
    }) {
        return Err(BackendError::environment(
            "protected-content manifest paths must be unique and canonically sorted",
        ));
    }
    for entry in entries {
        validate_absolute_path(entry.path())?;
        match entry {
            ManifestEntry::File { sha256, .. } if !valid_digest(sha256) => {
                return Err(BackendError::environment(
                    "protected-content manifest contains an invalid file digest",
                ));
            }
            ManifestEntry::Symlink { target, .. } if target.is_empty() => {
                return Err(BackendError::environment(
                    "protected-content manifest contains an empty symlink target",
                ));
            }
            _ => {}
        }
    }
    for required in [
        "/etc/ayni/mise.toml",
        "/usr/local/bin/ayni",
        "/usr/local/bin/mise",
    ] {
        if !entries
            .iter()
            .any(|entry| matches!(entry, ManifestEntry::File { path, .. } if path == required))
        {
            return Err(BackendError::environment(format!(
                "protected-content manifest is missing {required}"
            )));
        }
    }
    if !entries
        .iter()
        .any(|entry| entry.path().starts_with("/opt/ayni/mise/"))
    {
        return Err(BackendError::environment(
            "protected-content manifest is missing the installed runtime/tool tree",
        ));
    }
    Ok(())
}

fn parse_file_records(bytes: &[u8]) -> Result<Vec<ManifestEntry>, BackendError> {
    let mut entries = Vec::new();
    for record in nul_records(bytes, "file hash")? {
        if record.len() < 67
            || record.get(64..66) != Some(b"  ")
            || !record[..64]
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(BackendError::environment(
                "protected-content file hash output is malformed",
            ));
        }
        let path = std::str::from_utf8(&record[66..])
            .map_err(|_| BackendError::environment("protected-content path is not valid UTF-8"))?;
        validate_absolute_path(path)?;
        entries.push(ManifestEntry::File {
            path: path.into(),
            sha256: format!("sha256:{}", String::from_utf8_lossy(&record[..64])),
        });
    }
    Ok(entries)
}

fn parse_symlink_records(bytes: &[u8]) -> Result<Vec<ManifestEntry>, BackendError> {
    let records = nul_records(bytes, "symlink")?;
    if records.len() % 2 != 0 {
        return Err(BackendError::environment(
            "protected-content symlink output is malformed",
        ));
    }
    let mut entries = Vec::new();
    for pair in records.chunks_exact(2) {
        let path = std::str::from_utf8(pair[0]).map_err(|_| {
            BackendError::environment("protected-content symlink path is not valid UTF-8")
        })?;
        let target = std::str::from_utf8(pair[1]).map_err(|_| {
            BackendError::environment("protected-content symlink target is not valid UTF-8")
        })?;
        validate_absolute_path(path)?;
        if target.is_empty() {
            return Err(BackendError::environment(
                "protected-content symlink target is empty",
            ));
        }
        entries.push(ManifestEntry::Symlink {
            path: path.into(),
            target: target.into(),
        });
    }
    Ok(entries)
}

fn nul_records<'a>(bytes: &'a [u8], description: &str) -> Result<Vec<&'a [u8]>, BackendError> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    if !bytes.ends_with(&[0]) {
        return Err(BackendError::environment(format!(
            "protected-content {description} output is not NUL terminated"
        )));
    }
    Ok(bytes[..bytes.len() - 1].split(|byte| *byte == 0).collect())
}

fn validate_absolute_path(path: &str) -> Result<(), BackendError> {
    if !path.starts_with('/') || path.contains('\0') {
        return Err(BackendError::environment(format!(
            "protected-content inventory path is invalid: {path:?}"
        )));
    }
    Ok(())
}

fn validate_manifest(manifest: &str) -> Result<(), BackendError> {
    let header = format!("{{\"schema_version\":\"{MANIFEST_SCHEMA_VERSION}\"}}");
    if !manifest.ends_with('\n') {
        return Err(BackendError::environment(
            "protected-content manifest is not newline terminated",
        ));
    }
    let mut lines = manifest.lines();
    if lines.next() != Some(header.as_str()) {
        return Err(BackendError::environment(
            "protected-content manifest has an unsupported or malformed schema",
        ));
    }
    let entries = lines
        .map(|line| {
            serde_json::from_str::<ManifestEntry>(line).map_err(|error| {
                BackendError::environment(format!(
                    "protected-content manifest contains a malformed entry: {error}"
                ))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    validate_manifest_entries(&entries)?;
    if render_manifest(&entries) != manifest {
        return Err(BackendError::environment(
            "protected-content manifest is not canonically encoded",
        ));
    }
    Ok(())
}

fn required(name: &str) -> Result<String, BackendError> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            BackendError::environment(format!(
                "{name} is required to build a certified environment image"
            ))
        })
}

fn validate_key_id(value: &str) -> Result<(), BackendError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(BackendError::environment(format!(
            "{KEY_ID_ENV} must contain only ASCII letters, digits, '.', '-', or '_'"
        )));
    }
    Ok(())
}

fn signing_key(value: &str) -> Result<SigningKey, BackendError> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(BackendError::environment(format!(
            "{SIGNING_KEY_ENV} must be a 32-byte lowercase hexadecimal Ed25519 seed"
        )));
    }
    let mut seed = [0u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        seed[index] = u8::from_str_radix(std::str::from_utf8(chunk).expect("ASCII hex"), 16)
            .map_err(|_| {
                BackendError::environment(format!("{SIGNING_KEY_ENV} must be hexadecimal"))
            })?;
    }
    Ok(SigningKey::from_bytes(&seed))
}

fn valid_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(hash: u8, path: &str) -> Vec<u8> {
        format!("{}  {path}\0", format!("{hash:02x}").repeat(32)).into_bytes()
    }

    fn inventory() -> (Vec<u8>, Vec<u8>) {
        let mut files = file(3, "/usr/local/bin/mise");
        files.extend(file(1, "/opt/ayni/mise/installs/rust/bin/rustc"));
        files.extend(file(4, "/etc/ayni/mise.toml"));
        files.extend(file(2, "/usr/local/bin/ayni"));
        let links = b"/opt/ayni/mise/shims/rustc\0../bin/mise\0".to_vec();
        (files, links)
    }

    #[test]
    fn manifest_bytes_are_canonical_for_equivalent_inventory() {
        let (files, links) = inventory();
        let first = manifest_from_inventory(&files, &links).unwrap();

        let mut reordered = file(2, "/usr/local/bin/ayni");
        reordered.extend(file(4, "/etc/ayni/mise.toml"));
        reordered.extend(file(3, "/usr/local/bin/mise"));
        reordered.extend(file(1, "/opt/ayni/mise/installs/rust/bin/rustc"));
        let second = manifest_from_inventory(&reordered, &links).unwrap();

        assert_eq!(first, second);
        assert_eq!(
            sha256_fingerprint(first.as_bytes()),
            sha256_fingerprint(second.as_bytes())
        );
        assert!(first.starts_with("{\"schema_version\":\"1\"}\n"));
        assert!(first.contains("\"type\":\"file\""));
        assert!(first.contains("\"type\":\"symlink\""));
    }

    #[test]
    fn malformed_or_incomplete_inventory_fails_closed() {
        let (mut files, links) = inventory();
        files.pop();
        assert!(manifest_from_inventory(&files, &links).is_err());
        assert!(manifest_from_inventory(&file(1, "/usr/local/bin/ayni"), &[]).is_err());
        assert!(manifest_from_inventory(&inventory().0, b"path\0").is_err());
    }

    #[test]
    fn serialized_manifest_validation_rejects_noncanonical_or_malformed_entries() {
        let (files, links) = inventory();
        let manifest = manifest_from_inventory(&files, &links).unwrap();
        validate_manifest(&manifest).unwrap();

        let malformed = format!("{manifest}not-json\n");
        assert!(validate_manifest(&malformed).is_err());
        let duplicate = format!(
            "{}{}\n",
            manifest,
            manifest.lines().nth(1).expect("first entry")
        );
        assert!(validate_manifest(&duplicate).is_err());
        let noncanonical = manifest.replacen("{\"type\":\"file\",\"path\"", "{\"path\"", 1);
        assert!(validate_manifest(&noncanonical).is_err());
    }

    #[test]
    fn signing_inputs_are_strict_and_deterministic() {
        let first = SigningMaterial::from_values(&"07".repeat(32), "release-2026").unwrap();
        let second = SigningMaterial::from_values(&"07".repeat(32), "release-2026").unwrap();
        assert_eq!(first.key.to_bytes(), second.key.to_bytes());
        assert!(SigningMaterial::from_values(&"AA".repeat(32), "release-2026").is_err());
        assert!(SigningMaterial::from_values(&"07".repeat(31), "release-2026").is_err());
        assert!(SigningMaterial::from_values(&"07".repeat(32), "bad key").is_err());
    }
}
