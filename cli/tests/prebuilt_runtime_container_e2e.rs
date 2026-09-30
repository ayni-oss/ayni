#![cfg(target_os = "linux")]

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn docker_available() -> bool {
    Command::new("docker")
        .args(["info", "--format", "{{.Server.OSType}}"])
        .output()
        .is_ok_and(|output| output.status.success())
}

#[test]
fn container_runtime_marker_selects_the_read_only_mounted_source() {
    if !docker_available() {
        eprintln!("skipping container integration test: Docker daemon is unavailable");
        return;
    }

    let context = TempDir::new().expect("build context");
    let source = TempDir::new().expect("read-only source");
    fs::copy(env!("CARGO_BIN_EXE_ayni"), context.path().join("ayni")).expect("copy CLI");
    fs::write(
        context.path().join("runtime.json"),
        format!(
            r#"{{"certificate":{{"schema_version":"1","lock_fingerprint":"sha256:{}","platform":"linux/amd64","ayni_version":"{}","tool_inventory_digest":"sha256:{}","protected_content_root":"sha256:{}"}},"key_id":"release-2026","signature":"{}"}}"#,
            "a".repeat(64),
            env!("CARGO_PKG_VERSION"),
            "b".repeat(64),
            "c".repeat(64),
            "0".repeat(128),
        ),
    )
    .expect("runtime metadata");
    fs::write(
        context.path().join("Dockerfile"),
        "FROM debian:bookworm-slim\nCOPY ayni /usr/local/bin/ayni\nCOPY runtime.json /etc/ayni/runtime.json\nENTRYPOINT [\"ayni\"]\n",
    )
    .expect("Dockerfile");

    let tag = format!("ayni-prebuilt-runtime-test-{}", std::process::id());
    let build = Command::new("docker")
        .args(["build", "--tag", &tag])
        .arg(context.path())
        .output()
        .expect("build runtime image");
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let output = Command::new("docker")
        .args(["run", "--rm", "--volume"])
        .arg(format!("{}:/workspace:ro", source.path().display()))
        .args(["--workdir", "/workspace", &tag, "check"])
        .output()
        .expect("run prebuilt environment");
    let _ = Command::new("docker").args(["image", "rm", &tag]).output();

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("prebuilt environment source requires /workspace/.ayni.toml"),
        "{stderr}"
    );
    assert!(!stderr.contains("environment lock"), "{stderr}");
}
