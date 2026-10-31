use super::*;
use ayni_core::{
    Architecture, EnvironmentCertificateEnvelope, EnvironmentPlan, Language, Libc, OperatingSystem,
    ProvisioningBase, RepositoryIdentity, RequirementConfidence, RequirementSource,
    RuntimeRequirement, TargetEnvironment, TargetIdentity, TargetPlatform, VersionRequirement,
};
use ed25519_dalek::SigningKey;
use std::{error::Error, io};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const KEY_ID: &str = "offline-release";
const CLI_BYTES: &[u8] = b"independently trusted Ayni executable fixture";
struct Fixture {
    root: tempfile::TempDir,
    trust: EnvironmentCertificateTrustPolicy,
    lock: EnvironmentLock,
    content_root: String,
    ayni_digest: String,
    platform: String,
}
fn fixture() -> TestResult<Fixture> {
    let root = tempfile::tempdir()?;
    for (path, bytes) in [
        ("etc/ayni/mise.toml", b"[tools]\n".as_slice()),
        ("usr/local/bin/ayni", CLI_BYTES),
        (
            "usr/local/bin/ayni-entrypoint",
            b"fixture entrypoint".as_slice(),
        ),
        ("usr/local/bin/mise", b"fixture mise".as_slice()),
        ("opt/ayni/mise/installs/tool", b"fixture tool".as_slice()),
    ] {
        let path = root.path().join(path);
        fs::create_dir_all(path.parent().ok_or("fixture parent")?)?;
        fs::write(path, bytes)?;
    }
    let signing = SigningKey::from_bytes(&[7; 32]);
    let trust = EnvironmentCertificateTrustPolicy::new(BTreeMap::from([(
        KEY_ID.into(),
        ayni_core::lower_hex(signing.verifying_key().to_bytes()),
    )]))?;
    let lock = fixture_lock(&trust, env!("CARGO_PKG_VERSION"))?;
    let manifest =
        inventory::inventory(root.path()).map_err(|error| io::Error::other(error.message))?;
    let content_root = sha256_fingerprint(manifest.as_bytes());
    let platform = image_plan(&lock)
        .map_err(|error| io::Error::other(error.message))?
        .platform;
    Ok(Fixture {
        root,
        trust,
        lock,
        content_root,
        ayni_digest: sha256_fingerprint(CLI_BYTES),
        platform,
    })
}
fn fixture_lock(
    trust: &EnvironmentCertificateTrustPolicy,
    version: &str,
) -> TestResult<EnvironmentLock> {
    let source = RequirementSource::new(
        "declared",
        ".ayni.toml",
        None::<String>,
        RequirementConfidence::Exact,
    )?;
    let target = TargetEnvironment {
        target: TargetIdentity::new(Language::Rust, ".")?,
        workspace: None,
        package: None,
        runtimes: vec![RuntimeRequirement {
            runtime: "rust".into(),
            version: VersionRequirement::exact("1.94.0")?,
            components: vec![],
            targets: vec![],
            source,
        }],
        package_manager: None,
        signal_tools: vec![],
        system_requirements: vec![],
        dependency_locks: vec![],
    };
    let plan = EnvironmentPlan::new(
        RepositoryIdentity {
            name: "offline-fixture".into(),
            contract_digest: sha256_fingerprint(b"policy"),
        },
        vec![Architecture::Amd64, Architecture::Arm64]
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
    )?
    .resolve()?;
    Ok(EnvironmentLock::from_resolved_plan(
        &plan,
        version,
        "2026.6.14",
        ProvisioningBase {
            reference: "docker.io/library/debian:bookworm-slim".into(),
            digest: sha256_fingerprint(b"base"),
            variant: "debian".into(),
            mise_version: "2025.2.4".into(),
        },
        trust,
        ".ayni.toml",
        &BTreeMap::from([(".ayni.toml".into(), sha256_fingerprint(b"policy"))]),
    )?)
}
fn expectations(fixture: &Fixture) -> CertificationExpectations<'_> {
    CertificationExpectations {
        platform: &fixture.platform,
        ayni_executable_digest: &fixture.ayni_digest,
        protected_content_root: &fixture.content_root,
    }
}
fn certify(fixture: &Fixture) -> TestResult<CertificationLayer> {
    certify_filesystem(
        fixture.root.path(),
        &fixture.lock,
        &fixture.trust,
        &expectations(fixture),
        KEY_ID,
        &"07".repeat(32),
    )
    .map_err(|error| io::Error::other(error.message).into())
}
fn write_layer(layer: &CertificationLayer, path: &Path) -> TestResult {
    layer
        .write(path)
        .map_err(|error| io::Error::other(error.message).into())
}

#[test]
fn detached_layer_round_trips_signed_certificate_and_canonical_protected_inventory() -> TestResult {
    let fixture = fixture()?;
    let first = certify(&fixture)?;
    let second = certify(&fixture)?;
    assert_eq!(first.report, second.report);
    assert_eq!(first.bytes, second.bytes);
    assert_eq!(first.report.schema_version, "1");
    assert_eq!(first.report.layer_digest, sha256_fingerprint(&first.bytes));
    assert_eq!(first.report.diff_id, first.report.layer_digest);
    assert_eq!(
        first.report.trusted_keys_fingerprint,
        fixture.trust.fingerprint()
    );
    let mut archive = tar::Archive::new(first.bytes.as_slice());
    let mut files = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        assert_eq!(entry.header().uid()?, 0);
        assert_eq!(entry.header().gid()?, 0);
        assert_eq!(entry.header().mode()?, 0o444);
        assert_eq!(entry.header().mtime()?, 0);
        assert!(entry.header().entry_type().is_file());
        let path = entry.path()?.to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        assert!(files.insert(path, bytes).is_none());
    }
    assert_eq!(files.len(), 2);
    let certificate = files
        .get("etc/ayni/runtime.json")
        .ok_or("certificate missing")?;
    assert_eq!(
        sha256_fingerprint(certificate),
        first.report.certificate_digest
    );
    let envelope: EnvironmentCertificateEnvelope = serde_json::from_slice(certificate)?;
    envelope.verify(&fixture.trust)?;
    assert_eq!(envelope.key_id, KEY_ID);
    assert_eq!(
        envelope.certificate.lock_fingerprint,
        fixture.lock.fingerprint()
    );
    assert_eq!(envelope.certificate.platform, first.report.platform);
    let manifest = files
        .get("etc/ayni/protected-content.manifest")
        .ok_or("manifest missing")?;
    assert_eq!(
        sha256_fingerprint(manifest),
        first.report.protected_content_root
    );
    assert_eq!(
        envelope.certificate.protected_content_root,
        first.report.protected_content_root
    );
    assert_eq!(
        std::str::from_utf8(manifest)?,
        inventory::inventory(fixture.root.path())
            .map_err(|error| io::Error::other(error.message))?
    );
    assert!(
        !first
            .bytes
            .windows(64)
            .any(|window| window == "07".repeat(32).as_bytes())
    );
    assert_eq!(
        first.report.labels[image::IMAGE_OWNER_LABEL],
        image::IMAGE_OWNER_VALUE
    );
    assert_eq!(
        first.report.labels[certificate::CERTIFICATE_KEY_ID_LABEL],
        KEY_ID
    );
    Ok(())
}

#[test]
fn certification_rejects_unreviewed_trust_wrong_signer_and_unexpected_executable() -> TestResult {
    let fixture = fixture()?;
    for (expected, key_id, seed) in [
        (
            sha256_fingerprint(b"wrong executable"),
            KEY_ID,
            "07".repeat(32),
        ),
        (sha256_fingerprint(CLI_BYTES), KEY_ID, "08".repeat(32)),
        (
            sha256_fingerprint(CLI_BYTES),
            "untrusted-key",
            "07".repeat(32),
        ),
    ] {
        assert!(
            certify_filesystem(
                fixture.root.path(),
                &fixture.lock,
                &fixture.trust,
                &CertificationExpectations {
                    ayni_executable_digest: &expected,
                    ..expectations(&fixture)
                },
                key_id,
                &seed
            )
            .is_err()
        );
    }
    let other_platform = if fixture.platform == "linux/arm64" {
        "linux/amd64"
    } else {
        "linux/arm64"
    };
    assert!(
        certify_filesystem(
            fixture.root.path(),
            &fixture.lock,
            &fixture.trust,
            &CertificationExpectations {
                platform: other_platform,
                ..expectations(&fixture)
            },
            KEY_ID,
            &"07".repeat(32)
        )
        .is_err()
    );
    let empty = EnvironmentCertificateTrustPolicy::new(BTreeMap::new())?;
    assert!(
        certify_filesystem(
            fixture.root.path(),
            &fixture.lock,
            &empty,
            &expectations(&fixture),
            KEY_ID,
            &"07".repeat(32)
        )
        .is_err()
    );
    let altered = fixture_lock(&empty, env!("CARGO_PKG_VERSION"))?;
    assert!(
        certify_filesystem(
            fixture.root.path(),
            &altered,
            &fixture.trust,
            &expectations(&fixture),
            KEY_ID,
            &"07".repeat(32)
        )
        .is_err()
    );
    let unsupported = fixture_lock(&fixture.trust, "0.0.0")?;
    assert!(
        certify_filesystem(
            fixture.root.path(),
            &unsupported,
            &fixture.trust,
            &expectations(&fixture),
            KEY_ID,
            &"07".repeat(32)
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn unsigned_key_identifier_cannot_become_a_signing_authority() -> TestResult {
    let fixture = fixture()?;
    let signing = SigningKey::from_bytes(&[7; 32]);
    let trust = EnvironmentCertificateTrustPolicy::new(BTreeMap::from([(
        "unsigned".into(),
        ayni_core::lower_hex(signing.verifying_key().to_bytes()),
    )]))?;
    let lock = fixture_lock(&trust, env!("CARGO_PKG_VERSION"))?;
    let result = certify_filesystem(
        fixture.root.path(),
        &lock,
        &trust,
        &expectations(&fixture),
        "unsigned",
        &"07".repeat(32),
    );
    assert!(result.is_err_and(|error| error.message.contains("reserved")));
    Ok(())
}

#[test]
fn changed_protected_content_changes_evidence_and_cli_tampering_blocks_certification() -> TestResult
{
    let mut fixture = fixture()?;
    let original = certify(&fixture)?;
    fs::write(
        fixture.root.path().join("opt/ayni/mise/installs/tool"),
        b"changed tool",
    )?;
    assert!(certify(&fixture).is_err());
    let reviewed = inventory::inventory(fixture.root.path())
        .map_err(|error| io::Error::other(error.message))?;
    fixture.content_root = sha256_fingerprint(reviewed.as_bytes());
    let changed = certify(&fixture)?;
    assert_ne!(
        original.report.protected_content_root,
        changed.report.protected_content_root
    );
    assert_ne!(original.report.layer_digest, changed.report.layer_digest);
    fs::write(
        fixture.root.path().join("usr/local/bin/ayni"),
        b"untrusted replacement",
    )?;
    assert!(certify(&fixture).is_err());
    Ok(())
}

#[test]
fn atomic_output_retries_identical_layer_and_preserves_conflicting_files() -> TestResult {
    let fixture = fixture()?;
    let layer = certify(&fixture)?;
    let output = tempfile::tempdir()?;
    let path = output.path().join("certification.tar");
    write_layer(&layer, &path)?;
    write_layer(&layer, &path)?;
    assert_eq!(fs::read(&path)?, layer.bytes);
    fs::write(&path, b"conflicting retained output")?;
    assert!(layer.write(&path).is_err());
    assert_eq!(fs::read(&path)?, b"conflicting retained output");
    assert_eq!(fs::read_dir(output.path())?.count(), 1);
    Ok(())
}

#[test]
fn exact_retry_recovers_a_complete_output_with_an_interrupted_temporary_hardlink() -> TestResult {
    let fixture = fixture()?;
    let layer = certify(&fixture)?;
    let output = tempfile::tempdir()?;
    let path = output.path().join("certification.tar");
    write_layer(&layer, &path)?;
    let pending = output.path().join(".interrupted-publication");
    fs::hard_link(&path, &pending)?;
    write_layer(&layer, &path)?;
    assert_eq!(fs::read(&path)?, layer.bytes);
    assert_eq!(fs::read(&pending)?, layer.bytes);
    fs::write(&pending, b"conflicting linked content")?;
    assert!(layer.write(&path).is_err());
    assert_eq!(fs::read(&path)?, b"conflicting linked content");
    Ok(())
}

#[cfg(unix)]
#[test]
fn output_and_protected_file_symlinks_are_rejected_without_following_them() -> TestResult {
    use std::os::unix::fs::symlink;
    let fixture = fixture()?;
    let layer = certify(&fixture)?;
    let output = tempfile::tempdir()?;
    let target = output.path().join("retained.tar");
    write_layer(&layer, &target)?;
    let path = output.path().join("link.tar");
    symlink(&target, &path)?;
    assert!(layer.write(&path).is_err());
    assert_eq!(fs::read(&target)?, layer.bytes);
    let executable = fixture.root.path().join("usr/local/bin/ayni");
    fs::remove_file(&executable)?;
    symlink(&target, &executable)?;
    assert!(certify(&fixture).is_err());
    Ok(())
}

#[test]
fn expected_platform_is_checked_before_candidate_access_or_signing_material() -> TestResult {
    let fixture = fixture()?;
    let platform = if fixture.platform == "linux/arm64" {
        "linux/amd64"
    } else {
        "linux/arm64"
    };
    let error = certify_filesystem(
        &fixture.root.path().join("absent-snapshot"),
        &fixture.lock,
        &fixture.trust,
        &CertificationExpectations {
            platform,
            ..expectations(&fixture)
        },
        KEY_ID,
        "invalid-signing-material",
    )
    .err()
    .ok_or("wrong platform accepted")?;
    assert_eq!(error.kind, crate::BackendErrorKind::Input);
    assert!(error.message.contains("expected platform"));
    Ok(())
}
