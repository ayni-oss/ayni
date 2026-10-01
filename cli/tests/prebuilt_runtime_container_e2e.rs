#![cfg(unix)]

use ayni_core::{EnvironmentCertificateEnvelope, sha256_fingerprint};
use ayni_environment::ProtectedManifestEntry;
use ed25519_dalek::SigningKey;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

static IMAGE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

struct DerivedImages(Vec<String>);

impl DerivedImages {
    fn track(&mut self, image: String) -> String {
        self.0.push(image.clone());
        image
    }
}

impl Drop for DerivedImages {
    fn drop(&mut self) {
        for image in self.0.iter().rev() {
            let _ = Command::new("docker")
                .args(["image", "rm", "--force", image])
                .output();
        }
    }
}

fn required(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => Some(value),
        _ => {
            eprintln!("skipping certified runtime acceptance test: {name} is not set");
            None
        }
    }
}

fn command_output(command: &mut Command, description: &str) -> Output {
    command
        .output()
        .unwrap_or_else(|error| panic!("failed to {description}: {error}"))
}

fn require_success(output: &Output, description: &str) {
    assert!(
        output.status.success(),
        "{description} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn host_identity() -> String {
    let uid = command_output(Command::new("id").arg("-u"), "read host uid");
    let gid = command_output(Command::new("id").arg("-g"), "read host gid");
    format!(
        "{}:{}",
        String::from_utf8_lossy(&uid.stdout).trim(),
        String::from_utf8_lossy(&gid.stdout).trim()
    )
}

fn clear_quality_artifact(source: &Path) {
    let _ = fs::remove_dir_all(source.join(".ayni/last"));
}

fn run_check(image: &str, source: &Path) -> Output {
    clear_quality_artifact(source);
    let source = source.canonicalize().expect("canonical source");
    command_output(
        Command::new("docker")
            .args([
                "run",
                "--rm",
                "--network",
                "none",
                "--read-only",
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges",
                "--user",
                &host_identity(),
                "--mount",
                &format!(
                    "type=bind,source={},target=/workspace,readonly",
                    source.display()
                ),
                "--tmpfs",
                "/tmp:rw,exec,nosuid,size=1g",
                "--env",
                "HOME=/tmp/home",
                "--env",
                "XDG_STATE_HOME=/tmp/home/.local/state",
                "--env",
                "CARGO_TARGET_DIR=/tmp/target",
                "--workdir",
                "/workspace",
                "--entrypoint",
                "/bin/sh",
                image,
                "-c",
                "mkdir -p /tmp/home /tmp/target && exec /usr/local/bin/ayni check --config .ayni.toml --output json",
            ]),
        "run certified environment check",
    )
}

fn assert_rejected(output: &Output, expected: &str) {
    assert!(
        !output.status.success(),
        "tampered runtime unexpectedly passed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(expected),
        "expected {expected:?} in {stderr:?}"
    );
    assert!(
        !stderr.contains("running language="),
        "quality work started before runtime rejection: {stderr}"
    );
}

fn copy_from_image(image: &str, source: &str, destination: &Path) {
    let create = command_output(
        Command::new("docker").args(["create", image]),
        "create image extraction container",
    );
    require_success(&create, "create image extraction container");
    let container = String::from_utf8(create.stdout)
        .expect("container id is UTF-8")
        .trim()
        .to_owned();
    let copy = command_output(
        Command::new("docker").args([
            "cp",
            &format!("{container}:{source}"),
            &destination.display().to_string(),
        ]),
        "copy protected file from image",
    );
    let _ = Command::new("docker")
        .args(["rm", "--force", &container])
        .output();
    require_success(&copy, "copy protected file from image");
}

fn next_tag() -> String {
    format!(
        "ayni-prebuilt-runtime-acceptance-{}-{}",
        std::process::id(),
        IMAGE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

fn build_derived(
    base: &str,
    certificate: Option<&Path>,
    manifest: Option<&Path>,
    instructions: &str,
) -> String {
    let context = TempDir::new().expect("derived image context");
    let mut dockerfile = String::from("ARG BASE\nFROM ${BASE}\nUSER 0:0\n");
    if let Some(path) = certificate {
        fs::copy(path, context.path().join("runtime.json")).expect("stage certificate");
        dockerfile.push_str("COPY --chown=0:0 --chmod=0444 runtime.json /etc/ayni/runtime.json\n");
    }
    if let Some(path) = manifest {
        fs::copy(path, context.path().join("protected-content.manifest")).expect("stage manifest");
        dockerfile.push_str("COPY --chown=0:0 --chmod=0444 protected-content.manifest /etc/ayni/protected-content.manifest\n");
    }
    dockerfile.push_str(instructions);
    dockerfile.push_str("\nUSER 10001:10001\n");
    fs::write(context.path().join("Dockerfile"), dockerfile).expect("write Dockerfile");
    let tag = next_tag();
    let build = command_output(
        Command::new("docker")
            .args([
                "build",
                "--provenance=false",
                "--build-arg",
                &format!("BASE={base}"),
                "--tag",
                &tag,
            ])
            .arg(context.path()),
        "build tampered runtime image",
    );
    require_success(&build, "build tampered runtime image");
    tag
}

fn decode_seed(value: &str) -> SigningKey {
    assert_eq!(value.len(), 64, "test signing seed must be 32-byte hex");
    let mut seed = [0_u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        seed[index] = u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16)
            .expect("test signing seed is hexadecimal");
    }
    SigningKey::from_bytes(&seed)
}

fn write_certificate(path: &Path, envelope: &EnvironmentCertificateEnvelope) {
    let mut bytes = serde_json::to_vec(envelope).expect("serialize certificate");
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write certificate");
}

fn resign(
    envelope: &EnvironmentCertificateEnvelope,
    key: &SigningKey,
) -> EnvironmentCertificateEnvelope {
    EnvironmentCertificateEnvelope::sign(envelope.certificate.clone(), envelope.key_id.clone(), key)
        .expect("sign test certificate")
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied source directory");
    for entry in fs::read_dir(source).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let file_type = entry.file_type().expect("read source entry type");
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &target);
        } else if file_type.is_file() {
            fs::copy(entry.path(), target).expect("copy source file");
        } else if file_type.is_symlink() {
            std::os::unix::fs::symlink(
                fs::read_link(entry.path()).expect("read source symlink"),
                target,
            )
            .expect("copy source symlink");
        }
    }
}

#[test]
fn certified_environment_accepts_valid_runtime_and_rejects_every_identity_mismatch() {
    let Some(image) = required("AYNI_PREBUILT_TEST_IMAGE") else {
        return;
    };
    let Some(source) = required("AYNI_PREBUILT_TEST_SOURCE") else {
        return;
    };
    let Some(seed) = required("AYNI_PREBUILT_TEST_SIGNING_KEY") else {
        return;
    };
    let source = PathBuf::from(source);
    assert!(source.is_dir(), "test source is missing");
    let signing_key = decode_seed(&seed);
    let extracted = TempDir::new().expect("extracted protected content");
    let certificate_path = extracted.path().join("runtime.json");
    let manifest_path = extracted.path().join("protected-content.manifest");
    copy_from_image(&image, "/etc/ayni/runtime.json", &certificate_path);
    copy_from_image(
        &image,
        "/etc/ayni/protected-content.manifest",
        &manifest_path,
    );
    let original_bytes = fs::read(&certificate_path).expect("read certificate");
    let original: EnvironmentCertificateEnvelope =
        serde_json::from_slice(&original_bytes).expect("parse certificate");
    let mut images = DerivedImages(Vec::new());

    let success = run_check(&image, &source);
    require_success(&success, "check in valid certified environment");
    let evidence: serde_json::Value =
        serde_json::from_slice(&success.stdout).expect("successful check emits JSON evidence");
    assert_eq!(evidence["aggregate"]["status"], "pass");
    assert!(evidence["prebuilt_runtime"].is_object());
    assert!(
        evidence["tool_versions"]
            .as_array()
            .is_some_and(|versions| !versions.is_empty()),
        "successful evidence must retain locked tool versions"
    );
    assert!(
        !source.join(".ayni/last/signals.json").exists(),
        "direct prebuilt checks must not mutate read-only source evidence"
    );

    let mut invalid_signature = original.clone();
    let replacement = if invalid_signature.signature.starts_with("00") {
        "01"
    } else {
        "00"
    };
    invalid_signature.signature.replace_range(..2, replacement);
    let path = extracted.path().join("invalid-signature.json");
    write_certificate(&path, &invalid_signature);
    let altered = images.track(build_derived(&image, Some(&path), None, ""));
    assert_rejected(&run_check(&altered, &source), "signature is invalid");

    let mut untrusted = original.clone();
    untrusted.key_id = String::from("untrusted-test-key");
    let path = extracted.path().join("untrusted.json");
    write_certificate(&path, &untrusted);
    let altered = images.track(build_derived(&image, Some(&path), None, ""));
    assert_rejected(&run_check(&altered, &source), "is not trusted");

    for (name, mutate) in [
        ("wrong-platform", ("platform", "linux/unsupported")),
        ("wrong-version", ("version", "999.0.0")),
    ] {
        let mut changed = original.clone();
        match mutate.0 {
            "platform" => changed.certificate.platform = mutate.1.into(),
            "version" => changed.certificate.ayni_version = mutate.1.into(),
            _ => unreachable!(),
        }
        changed = resign(&changed, &signing_key);
        let path = extracted.path().join(format!("{name}.json"));
        write_certificate(&path, &changed);
        let altered = images.track(build_derived(&image, Some(&path), None, ""));
        assert_rejected(&run_check(&altered, &source), "claims do not match");
    }

    let malformed = extracted.path().join("malformed.json");
    fs::write(&malformed, b"{}\n").expect("write malformed certificate");
    let altered = images.track(build_derived(&image, Some(&malformed), None, ""));
    assert_rejected(
        &run_check(&altered, &source),
        "malformed prebuilt environment metadata",
    );

    let noncanonical = extracted.path().join("noncanonical.json");
    let mut pretty = serde_json::to_vec_pretty(&original).expect("pretty certificate");
    pretty.push(b'\n');
    fs::write(&noncanonical, pretty).expect("write noncanonical certificate");
    let altered = images.track(build_derived(&image, Some(&noncanonical), None, ""));
    assert_rejected(&run_check(&altered, &source), "not canonically encoded");

    let oversized = extracted.path().join("oversized.json");
    fs::write(
        &oversized,
        vec![b'x'; ayni_environment::MAX_CERTIFICATE_BYTES as usize + 1],
    )
    .expect("write oversized certificate");
    let altered = images.track(build_derived(&image, Some(&oversized), None, ""));
    assert_rejected(
        &run_check(&altered, &source),
        "exceeds the supported size limit",
    );

    let mut overlong_key_id = original.clone();
    overlong_key_id.key_id = "k".repeat(ayni_environment::MAX_CERTIFICATE_KEY_ID_BYTES + 1);
    overlong_key_id = resign(&overlong_key_id, &signing_key);
    let path = extracted.path().join("overlong-key-id.json");
    write_certificate(&path, &overlong_key_id);
    let altered = images.track(build_derived(&image, Some(&path), None, ""));
    assert_rejected(
        &run_check(&altered, &source),
        "invalid certificate envelope",
    );

    let malformed_manifest = extracted.path().join("malformed.manifest");
    fs::write(
        &malformed_manifest,
        b"{\"schema_version\":\"1\"}\nnot-json\n",
    )
    .expect("write malformed manifest");
    let mut matching = original.clone();
    matching.certificate.protected_content_root =
        sha256_fingerprint(fs::read(&malformed_manifest).unwrap());
    matching = resign(&matching, &signing_key);
    let matching_path = extracted.path().join("malformed-manifest-certificate.json");
    write_certificate(&matching_path, &matching);
    let altered = images.track(build_derived(
        &image,
        Some(&matching_path),
        Some(&malformed_manifest),
        "",
    ));
    assert_rejected(&run_check(&altered, &source), "malformed entry");

    let original_manifest = fs::read_to_string(&manifest_path).expect("read protected manifest");
    let mut unsafe_entries =
        ayni_environment::parse_protected_manifest(&original_manifest).expect("parse manifest");
    let unsafe_target = unsafe_entries
        .iter_mut()
        .find_map(|entry| match entry {
            ProtectedManifestEntry::Symlink { target, .. } => Some(target),
            ProtectedManifestEntry::File { .. } => None,
        })
        .expect("certified fixture contains a protected symlink");
    *unsafe_target = String::from("/tmp/unsigned-tool");
    let mut unsafe_manifest = String::from("{\"schema_version\":\"1\"}\n");
    for entry in unsafe_entries {
        unsafe_manifest.push_str(&serde_json::to_string(&entry).expect("serialize manifest entry"));
        unsafe_manifest.push('\n');
    }
    let unsafe_manifest_path = extracted.path().join("unsafe-symlink.manifest");
    fs::write(&unsafe_manifest_path, unsafe_manifest.as_bytes()).expect("write unsafe manifest");
    let mut matching = original.clone();
    matching.certificate.protected_content_root = sha256_fingerprint(unsafe_manifest.as_bytes());
    matching = resign(&matching, &signing_key);
    let matching_path = extracted.path().join("unsafe-symlink-certificate.json");
    write_certificate(&matching_path, &matching);
    let altered = images.track(build_derived(
        &image,
        Some(&matching_path),
        Some(&unsafe_manifest_path),
        "",
    ));
    assert_rejected(
        &run_check(&altered, &source),
        "resolves outside protected roots",
    );

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN printf tampered >> /usr/local/bin/mise\n",
    ));
    assert_rejected(
        &run_check(&altered, &source),
        "does not match its manifest digest",
    );

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN printf unlisted > /opt/ayni/mise/unlisted-runtime-content\n",
    ));
    assert_rejected(&run_check(&altered, &source), "unlisted path");

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN mkfifo /opt/ayni/mise/unsupported-runtime-node\n",
    ));
    assert_rejected(&run_check(&altered, &source), "unsupported file type");

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN chmod o+w /usr/local/bin/mise\n",
    ));
    assert_rejected(
        &run_check(&altered, &source),
        "must be root-owned and not writable",
    );

    let altered = images.track(build_derived(&image, None, None, "RUN chmod o+w /etc\n"));
    assert_rejected(
        &run_check(&altered, &source),
        "must be root-owned and not writable",
    );

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN chmod o+w /home/ayni\n",
    ));
    assert_rejected(
        &run_check(&altered, &source),
        "must be root-owned and not writable",
    );

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN chmod u+s /usr/local/bin/mise\n",
    ));
    assert_rejected(&run_check(&altered, &source), "no special mode bits");

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN link=\"$(find /opt/ayni -type l -print -quit)\" && test -n \"$link\" && rm \"$link\" && ln -s /tmp/ayni-tampered \"$link\"\n",
    ));
    assert_rejected(
        &run_check(&altered, &source),
        "does not match its manifest target",
    );

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN chmod 0644 /etc/ayni/runtime.json\n",
    ));
    assert_rejected(&run_check(&altered, &source), "root-owned mode 0444");

    let altered = images.track(build_derived(
        &image,
        None,
        None,
        "RUN chmod 0644 /etc/ayni/protected-content.manifest\n",
    ));
    assert_rejected(&run_check(&altered, &source), "root-owned and mode 0444");

    let stale = TempDir::new().expect("stale source");
    copy_tree(&source, stale.path());
    let config = stale.path().join(".ayni.toml");
    let mut contents = fs::read_to_string(&config).expect("read copied config");
    contents.push_str("\n# source contract changed after locking\n");
    fs::write(config, contents).expect("make lock stale");
    assert_rejected(&run_check(&image, stale.path()), "lock is stale");
}
