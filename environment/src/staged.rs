//! Detached certification for an immutable, externally prepared filesystem snapshot.
//!
//! The caller owns source preparation, snapshot immutability and final OCI publication.
//! This module executes no candidate programs, tools, engine commands or network requests.
use crate::{BackendError, certificate, image, image_plan};
use ayni_core::{EnvironmentCertificateTrustPolicy, EnvironmentLock, sha256_fingerprint};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read as _, Write as _},
    path::Path,
};

mod inventory;
#[cfg(test)]
mod tests;

/// Public certification evidence. The final OCI manifest digest is bound externally
/// after this layer is applied to the independently selected candidate image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CertificationReport {
    pub schema_version: String,
    pub lock_fingerprint: String,
    pub platform: String,
    pub ayni_version: String,
    pub ayni_executable_digest: String,
    pub trusted_keys_fingerprint: String,
    pub certificate_digest: String,
    pub protected_content_root: String,
    pub key_id: String,
    pub layer_digest: String,
    pub diff_id: String,
    pub labels: BTreeMap<String, String>,
}

/// Independently reviewed identities retained outside candidate-writable storage.
/// Certification supports the signer's native Linux image architecture only.
#[derive(Clone, Copy, Debug)]
pub struct CertificationExpectations<'a> {
    pub platform: &'a str,
    pub ayni_executable_digest: &'a str,
    pub protected_content_root: &'a str,
}

/// An uncompressed, deterministic tar layer containing only the official
/// certificate and protected-content manifest. It contains no signing key.
pub struct CertificationLayer {
    pub report: CertificationReport,
    bytes: Vec<u8>,
}

/// Certify actual protected content without executing the candidate filesystem.
/// The caller must hold a read-only immutable snapshot throughout certification;
/// the two inventories detect changes but do not replace snapshot isolation.
///
/// # Errors
/// Reject unsupported lock/platform identity, absent or mismatched reviewed trust,
/// signer mismatch, unexpected executable bytes, unsafe filesystem content or mutation.
pub fn certify_filesystem(
    filesystem_root: &Path,
    lock: &EnvironmentLock,
    trust: &EnvironmentCertificateTrustPolicy,
    expectations: &CertificationExpectations<'_>,
    key_id: &str,
    signing_seed: &str,
) -> Result<CertificationLayer, BackendError> {
    let expected_ayni_digest = expectations.ayni_executable_digest;
    let expected_content_root = expectations.protected_content_root;
    validate_inputs(lock, trust, expected_ayni_digest)?;
    let plan = image_plan(lock)?;
    if expectations.platform != plan.platform {
        return Err(BackendError::input(
            "expected platform must match the certification host architecture",
        ));
    }
    if !crate::executor::valid_digest(expected_content_root) {
        return Err(BackendError::input(
            "expected protected-content root must be a canonical SHA-256 identity",
        ));
    }
    if key_id == "unsigned" {
        return Err(BackendError::input(
            "unsigned is reserved for metadata without a signing authority",
        ));
    }
    let signing = certificate::SigningMaterial::from_values(signing_seed, key_id)
        .map_err(|_| BackendError::input("invalid signing seed or certificate key identifier"))?;
    if !signing.matches_trust(trust) {
        return Err(BackendError::input(
            "signer does not match the reviewed repository trust policy",
        ));
    }
    let manifest = inventory::inventory(filesystem_root)?;
    validate_reviewed_manifest(&manifest, expectations)?;
    let certification = certificate::create(lock, &plan, manifest, Some(&signing))?;
    if inventory::inventory(filesystem_root)? != certification.manifest {
        return Err(BackendError::environment(
            "protected filesystem changed during certification",
        ));
    }
    let bytes = metadata_layer(&certification)?;
    let layer_digest = sha256_fingerprint(&bytes);
    let labels = BTreeMap::from([
        (
            image::IMAGE_OWNER_LABEL.into(),
            image::IMAGE_OWNER_VALUE.into(),
        ),
        (
            certificate::CERTIFICATE_SCHEMA_LABEL.into(),
            certification.identity.schema_version.clone(),
        ),
        (
            certificate::CERTIFICATE_KEY_ID_LABEL.into(),
            certification.identity.key_id.clone(),
        ),
        (
            certificate::PROTECTED_CONTENT_ROOT_LABEL.into(),
            certification.identity.protected_content_root.clone(),
        ),
    ]);
    Ok(CertificationLayer {
        report: CertificationReport {
            schema_version: "1".into(),
            lock_fingerprint: lock.fingerprint().into(),
            platform: plan.platform,
            ayni_version: env!("CARGO_PKG_VERSION").into(),
            ayni_executable_digest: expected_ayni_digest.into(),
            trusted_keys_fingerprint: trust.fingerprint(),
            certificate_digest: certification.identity.certificate_digest,
            protected_content_root: certification.identity.protected_content_root,
            key_id: certification.identity.key_id,
            diff_id: layer_digest.clone(),
            layer_digest,
            labels,
        },
        bytes,
    })
}

fn validate_reviewed_manifest(
    manifest: &str,
    expectations: &CertificationExpectations<'_>,
) -> Result<(), BackendError> {
    if sha256_fingerprint(manifest.as_bytes()) != expectations.protected_content_root {
        return Err(BackendError::input(
            "protected content does not match the independently reviewed root",
        ));
    }
    let entries = certificate::parse_protected_manifest(manifest)?;
    let executable_matches = entries.iter().any(|entry| {
        matches!(entry,
        certificate::ProtectedManifestEntry::File { path, sha256 }
            if path == "/usr/local/bin/ayni" && sha256 == expectations.ayni_executable_digest)
    });
    if !executable_matches {
        return Err(BackendError::input(
            "candidate Ayni executable does not match its independently trusted digest",
        ));
    }
    Ok(())
}

fn validate_inputs(
    lock: &EnvironmentLock,
    trust: &EnvironmentCertificateTrustPolicy,
    expected: &str,
) -> Result<(), BackendError> {
    if !crate::executor::valid_digest(expected) {
        return Err(BackendError::input(
            "expected Ayni executable digest must be a canonical SHA-256 identity",
        ));
    }
    if lock.ayni_version() != env!("CARGO_PKG_VERSION") {
        return Err(BackendError::input(
            "lock Ayni version does not match this certification implementation",
        ));
    }
    if trust.is_empty()
        || lock.certificate_trust_policy().schema_version
            != ayni_core::ENVIRONMENT_CERTIFICATE_SCHEMA_VERSION
        || lock.certificate_trust_policy().fingerprint != trust.fingerprint()
    {
        return Err(BackendError::input(
            "reviewed nonempty certificate trust must match the exact lock trust fingerprint",
        ));
    }
    let architecture = crate::lock::host_architecture()?;
    if !lock
        .platforms()
        .iter()
        .any(|platform| platform.architecture == architecture)
    {
        return Err(BackendError::input(
            "lock does not support the certification platform",
        ));
    }
    Ok(())
}

fn metadata_layer(certification: &certificate::Certification) -> Result<Vec<u8>, BackendError> {
    let mut archive = tar::Builder::new(Vec::new());
    for (path, bytes) in [
        (
            "etc/ayni/runtime.json",
            certification.certificate.as_bytes(),
        ),
        (
            "etc/ayni/protected-content.manifest",
            certification.manifest.as_bytes(),
        ),
    ] {
        let mut header = tar::Header::new_ustar();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mode(0o444);
        header.set_mtime(0);
        header.set_size(bytes.len() as u64);
        header.set_cksum();
        archive
            .append_data(&mut header, path, bytes)
            .map_err(|_| BackendError::execution("failed to encode certification layer"))?;
    }
    archive
        .into_inner()
        .map_err(|_| BackendError::execution("failed to finish certification layer"))
}

impl CertificationLayer {
    /// Atomically publish this exact layer without overwriting an existing artifact.
    /// An identical regular file is an idempotent retry. The parent directory must exist.
    ///
    /// # Errors
    /// Reject conflicting output, symlinks/non-files, interrupted reads or writes and
    /// unsupported filesystem durability operations. No signing material is written.
    pub fn write(&self, output: &Path) -> Result<(), BackendError> {
        let parent = output
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let parent = parent.canonicalize().map_err(|_| {
            BackendError::execution("certification output directory is unavailable")
        })?;
        let output = parent.join(
            output
                .file_name()
                .ok_or_else(|| BackendError::input("certification output must name a file"))?,
        );
        if !existing_matches(&output, &self.bytes)? {
            let mut temporary = tempfile::NamedTempFile::new_in(&parent)
                .map_err(|_| BackendError::execution("cannot create certification output"))?;
            temporary
                .write_all(&self.bytes)
                .and_then(|_| temporary.as_file().sync_all())
                .map_err(|_| BackendError::execution("cannot persist certification output"))?;
            if temporary.persist_noclobber(&output).is_err()
                && !existing_matches(&output, &self.bytes)?
            {
                return Err(BackendError::execution(
                    "cannot publish certification output atomically",
                ));
            }
        }
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| BackendError::execution("cannot sync certification output directory"))
    }
}

#[cfg(unix)]
fn existing_matches(path: &Path, expected: &[u8]) -> Result<bool, BackendError> {
    let Some(mut file) = open_existing(path)? else {
        return Ok(false);
    };
    let before = file
        .metadata()
        .map_err(|_| BackendError::execution("cannot inspect certification output"))?;
    if !before.is_file() || before.len() != expected.len() as u64 {
        return Err(BackendError::input(
            "certification output conflicts with the requested layer",
        ));
    }
    let mut bytes = Vec::new();
    std::io::Read::take(&mut file, expected.len() as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| BackendError::execution("cannot read certification output"))?;
    let after = file
        .metadata()
        .map_err(|_| BackendError::execution("cannot recheck certification output"))?;
    let named = fs::symlink_metadata(path)
        .map_err(|_| BackendError::execution("cannot recheck named certification output"))?;
    if !named.is_file()
        || !same_metadata(&named, &after)
        || !same_metadata(&before, &after)
        || bytes != expected
    {
        return Err(BackendError::input(
            "certification output conflicts with the requested layer",
        ));
    }
    Ok(true)
}
#[cfg(unix)]
fn open_existing(path: &Path) -> Result<Option<fs::File>, BackendError> {
    use std::os::unix::fs::OpenOptionsExt as _;
    match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(BackendError::input(
            "certification output is not an accessible regular file",
        )),
    }
}
#[cfg(unix)]
fn same_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt as _;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.mode() == right.mode()
        && left.nlink() == right.nlink()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
        && left.ctime() == right.ctime()
        && left.ctime_nsec() == right.ctime_nsec()
}
#[cfg(not(unix))]
fn existing_matches(_: &Path, _: &[u8]) -> Result<bool, BackendError> {
    Err(BackendError::environment(
        "atomic certification output requires a supported Unix filesystem",
    ))
}
