//! Exercise offline signing through the executable with no tools on PATH.
#![cfg(all(unix, any(target_arch = "aarch64", target_arch = "x86_64")))]

use ayni_core::{
    Architecture, EnvironmentCertificateEnvelope, EnvironmentCertificateTrustPolicy,
    EnvironmentLock, EnvironmentPlan, Language, Libc, OperatingSystem, RepositoryIdentity,
    RequirementConfidence, RequirementSource, RuntimeRequirement, TargetEnvironment,
    TargetIdentity, TargetPlatform, VersionRequirement, sha256_fingerprint,
};
use ed25519_dalek::SigningKey;
use std::{collections::BTreeMap, error::Error, fs, io::Read, path::Path, process::Command};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn executable_certifies_offline_retries_exactly_and_rejects_changed_content() -> TestResult {
    let fixture = tempfile::tempdir()?;
    let source = fixture.path().join("source");
    let candidate = fixture.path().join("candidate");
    let empty_path = fixture.path().join("no-tools");
    fs::create_dir_all(&source)?;
    fs::create_dir_all(&candidate)?;
    fs::create_dir_all(&empty_path)?;
    let (trust, lock) = prepare_source(&source)?;
    let (manifest, expected_cli) = prepare_candidate(&candidate)?;
    let expected_root = sha256_fingerprint(manifest.as_bytes());
    let key = fixture.path().join("synthetic.seed");
    fs::write(&key, "07".repeat(32))?;
    let output = fixture.path().join("metadata.tar");
    let platform = if cfg!(target_arch = "aarch64") {
        "linux/arm64"
    } else {
        "linux/amd64"
    };
    let invoke = || {
        Command::new(env!("CARGO_BIN_EXE_ayni"))
            .args(["env", "certify", "--repo-root"])
            .arg(&source)
            .arg("--filesystem-root")
            .arg(&candidate)
            .args(["--platform", platform])
            .args(["--expected-ayni-sha256", &expected_cli])
            .args(["--expected-content-root", &expected_root])
            .args(["--key-id", "offline", "--signing-key-file"])
            .arg(&key)
            .arg("--output")
            .arg(&output)
            .env("PATH", &empty_path)
            .env("AYNI_ENV_CERTIFICATE_SIGNING_KEY", "invalid ambient seed")
            .env("AYNI_ENV_CERTIFICATE_KEY_ID", "wrong-ambient-authority")
            .output()
    };
    let first = invoke()?;
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&first.stdout)?;
    let original = fs::read(&output)?;
    assert_eq!(report["layer_digest"], sha256_fingerprint(&original));
    assert_eq!(report["protected_content_root"], expected_root);
    assert_eq!(report["platform"], platform);
    assert_eq!(report["key_id"], "offline");
    verify_layer(&original, &trust, &lock, &manifest)?;
    verify_recovery(invoke, &output, &candidate, &first, &original)
}

fn verify_recovery(
    invoke: impl Fn() -> std::io::Result<std::process::Output>,
    output: &Path,
    candidate: &Path,
    first: &std::process::Output,
    original: &[u8],
) -> TestResult {
    let retry = invoke()?;
    assert!(retry.status.success());
    assert_eq!(retry.stdout, first.stdout);
    assert_eq!(fs::read(output)?, original);
    fs::write(
        candidate.join("opt/ayni/mise/installs/tool"),
        b"changed tool",
    )?;
    let rejected = invoke()?;
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("reviewed root"));
    assert_eq!(fs::read(output)?, original);
    for bytes in [
        &first.stdout,
        &first.stderr,
        &rejected.stdout,
        &rejected.stderr,
        original,
    ] {
        assert!(
            !bytes
                .windows(64)
                .any(|window| window == "07".repeat(32).as_bytes())
        );
    }
    Ok(())
}

fn prepare_source(
    source: &Path,
) -> Result<(EnvironmentCertificateTrustPolicy, EnvironmentLock), Box<dyn Error>> {
    let signing = SigningKey::from_bytes(&[7; 32]);
    let public_key = ayni_core::lower_hex(signing.verifying_key().to_bytes());
    let policy = format!(
        "[checks]\ntest = false\ncoverage = false\nsize = true\n\
         [environment.certificate.trusted_keys]\noffline = \"{public_key}\"\n"
    );
    fs::write(source.join(".ayni.toml"), &policy)?;
    let trust =
        EnvironmentCertificateTrustPolicy::new(BTreeMap::from([("offline".into(), public_key)]))?;
    let lock = fixture_lock(&policy, &trust)?;
    fs::write(source.join(".ayni.lock"), lock.canonical_json()?)?;

    Ok((trust, lock))
}

fn prepare_candidate(candidate: &Path) -> Result<(String, String), Box<dyn Error>> {
    let files = BTreeMap::from([
        ("/etc/ayni/mise.toml", b"[tools]\n".as_slice()),
        ("/opt/ayni/mise/installs/tool", b"reviewed tool".as_slice()),
        (
            "/usr/local/bin/ayni",
            b"must never execute this candidate".as_slice(),
        ),
        (
            "/usr/local/bin/ayni-entrypoint",
            b"must never run entrypoint".as_slice(),
        ),
        ("/usr/local/bin/mise", b"must never run mise".as_slice()),
    ]);
    let mut manifest = String::from("{\"schema_version\":\"1\"}\n");
    for (path, bytes) in &files {
        let actual = candidate.join(path.trim_start_matches('/'));
        fs::create_dir_all(actual.parent().ok_or("fixture parent")?)?;
        fs::write(actual, bytes)?;
        let entry = ayni_environment::ProtectedManifestEntry::File {
            path: (*path).into(),
            sha256: sha256_fingerprint(bytes),
        };
        manifest.push_str(&serde_json::to_string(&entry)?);
        manifest.push('\n');
    }
    let expected_cli = sha256_fingerprint(files["/usr/local/bin/ayni"]);
    Ok((manifest, expected_cli))
}

fn verify_layer(
    original: &[u8],
    trust: &EnvironmentCertificateTrustPolicy,
    lock: &EnvironmentLock,
    manifest: &str,
) -> TestResult {
    let mut archive = tar::Archive::new(original);
    let mut entries = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        assert_eq!(entry.header().mode()?, 0o444);
        assert_eq!(entry.header().uid()?, 0);
        let name = entry.path()?.into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        entries.insert(name, bytes);
    }
    assert_eq!(entries.len(), 2);
    let envelope: EnvironmentCertificateEnvelope =
        serde_json::from_slice(&entries[Path::new("etc/ayni/runtime.json")])?;
    envelope.verify(trust)?;
    assert_eq!(envelope.certificate.lock_fingerprint, lock.fingerprint());
    assert_eq!(
        entries[Path::new("etc/ayni/protected-content.manifest")],
        manifest.as_bytes()
    );
    Ok(())
}

fn fixture_lock(
    policy: &str,
    trust: &EnvironmentCertificateTrustPolicy,
) -> Result<EnvironmentLock, Box<dyn Error>> {
    let target = TargetEnvironment {
        target: TargetIdentity::new(Language::Rust, ".")?,
        workspace: None,
        package: None,
        runtimes: vec![RuntimeRequirement {
            runtime: "rust".into(),
            version: VersionRequirement::exact("1.97.1")?,
            components: vec![],
            targets: vec![],
            source: RequirementSource::new(
                "policy",
                ".ayni.toml",
                None::<String>,
                RequirementConfidence::Exact,
            )?,
        }],
        package_manager: None,
        signal_tools: vec![],
        system_requirements: vec![],
        dependency_locks: vec![],
    };
    let plan = EnvironmentPlan::new(
        RepositoryIdentity {
            name: "offline-certification".into(),
            contract_digest: sha256_fingerprint(policy),
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
    )?
    .resolve()?;
    let base = ayni_environment::resolve_provisioning_base(env!("CARGO_PKG_VERSION"), None)
        .map_err(|error| format!("{error:?}"))?;
    Ok(EnvironmentLock::from_resolved_plan(
        &plan,
        env!("CARGO_PKG_VERSION"),
        base.mise_version.clone(),
        base,
        trust,
        ".ayni.toml",
        &BTreeMap::from([(".ayni.toml".into(), sha256_fingerprint(policy))]),
    )?)
}
