//! Execute in a certified image without a nested container engine.
use super::{CapturedLaunch, LaunchAuthorization, ReadOnlyInput, WORKSPACE};
use crate::{BackendError, prebuilt};
use ayni_adapters_common::exec::{DEFAULT_TOOL_TIMEOUT, run_command};
use ayni_core::{DependencyPreparationPlan, EnvironmentLock, PreparationOutputMode};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// A launcher must establish isolation before starting the embedded runner.
/// These checks validate observable Linux posture, not the launcher's identity.
pub fn validate_posture(
    source: &Path,
    lock: &EnvironmentLock,
    authorization: LaunchAuthorization,
) -> Result<(), BackendError> {
    super::validate_launch_authorization(lock.capabilities(), authorization)?;
    #[cfg(target_os = "linux")]
    {
        let status = fs::read_to_string("/proc/self/status").map_err(io_error)?;
        validate_process_status(&status)?;
        require_read_only(Path::new("/"))?;
        require_read_only(source)?;
        if lock.capabilities().network == ayni_core::NetworkAccess::None {
            validate_disabled_network()?;
        }
        if lock.capabilities().docker == ayni_core::DockerAccess::None
            && Path::new("/var/run/docker.sock").exists()
        {
            return Err(BackendError::environment(
                "prebuilt quality execution does not authorize the mounted Docker socket",
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = source;
        Err(BackendError::environment(
            "prebuilt quality execution requires Linux",
        ))
    }
}

#[cfg(target_os = "linux")]
fn validate_disabled_network() -> Result<(), BackendError> {
    for entry in fs::read_dir("/sys/class/net").map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        if !entry.path().is_dir() || entry.file_name() == "lo" {
            continue;
        }
        let flags = fs::read_to_string(entry.path().join("flags")).map_err(io_error)?;
        let flags =
            u32::from_str_radix(flags.trim().trim_start_matches("0x"), 16).map_err(io_error)?;
        if flags & 1 != 0 {
            return Err(BackendError::environment(
                "prebuilt quality execution requires disabled networking; launch with --network none",
            ));
        }
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn validate_process_status(status: &str) -> Result<(), BackendError> {
    let fields = status
        .lines()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key, value.trim()))
        .collect::<BTreeMap<_, _>>();
    let identity = ["Uid", "Gid"].iter().all(|key| {
        fields.get(key).is_some_and(|value| {
            value.split_whitespace().collect::<Vec<_>>() == ["10001", "10001", "10001", "10001"]
        })
    });
    let no_caps = ["CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb"]
        .iter()
        .all(|key| {
            fields
                .get(key)
                .is_some_and(|value| u64::from_str_radix(value, 16) == Ok(0))
        });
    if !identity || !no_caps || fields.get("NoNewPrivs") != Some(&"1") {
        return Err(BackendError::environment(
            "prebuilt quality execution requires user 10001:10001, --cap-drop ALL and --security-opt no-new-privileges",
        ));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn require_read_only(path: &Path) -> Result<(), BackendError> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(path.as_os_str().as_bytes()).map_err(io_error)?;
    let mut status = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: the NUL-terminated path and writable statvfs buffer are valid.
    if unsafe { libc::statvfs(name.as_ptr(), status.as_mut_ptr()) } != 0 {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    // SAFETY: statvfs initialized the buffer on success.
    if unsafe { status.assume_init() }.f_flag & libc::ST_RDONLY == 0 {
        return Err(BackendError::environment(format!(
            "prebuilt runtime and source must be read-only: {}",
            path.display()
        )));
    }
    Ok(())
}

fn io_error(error: impl std::fmt::Display) -> BackendError {
    BackendError::execution(format!("portable workspace: {error}"))
}

fn execute(
    cwd: &Path,
    program: &str,
    args: &[String],
) -> Result<std::process::Output, BackendError> {
    let output = run_command(cwd, program, args, DEFAULT_TOOL_TIMEOUT).map_err(io_error)?;
    if !output.status.success() {
        return Err(BackendError::execution(format!(
            "portable preparation failed: {}",
            crate::concise_output(&output.stderr)
        )));
    }
    Ok(output)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), BackendError> {
    fs::create_dir_all(destination).map_err(io_error)?;
    execute(
        Path::new("/"),
        "/bin/cp",
        &[
            "-RP".into(),
            "--".into(),
            format!("{}/.", source.display()),
            destination.display().to_string(),
        ],
    )?;
    execute(
        Path::new("/"),
        "/usr/bin/find",
        &[
            destination.display().to_string(),
            "-mindepth".into(),
            if destination == Path::new(WORKSPACE) {
                "1"
            } else {
                "0"
            }
            .into(),
            "(".into(),
            "-type".into(),
            "d".into(),
            "-o".into(),
            "-type".into(),
            "f".into(),
            ")".into(),
            "-exec".into(),
            "/bin/chmod".into(),
            "u+rwX".into(),
            "--".into(),
            "{}".into(),
            "+".into(),
        ],
    )?;
    Ok(())
}

struct Workspace;
impl Workspace {
    fn acquire() -> Result<Self, BackendError> {
        let mountinfo = fs::read_to_string("/proc/self/mountinfo").map_err(io_error)?;
        if !mountinfo.lines().any(|line| {
            line.split_whitespace().nth(4) == Some(WORKSPACE) && line.contains(" - tmpfs ")
        }) {
            return Err(BackendError::environment(
                "prebuilt execution requires an empty /workspace tmpfs",
            ));
        }
        if fs::read_dir(WORKSPACE).map_err(io_error)?.next().is_some() {
            return Err(BackendError::environment(
                "prebuilt /workspace must be empty; use a fresh container for each quality invocation",
            ));
        }
        fs::create_dir(Path::new(WORKSPACE).join(".ayni-session")).map_err(io_error)?;
        Ok(Self)
    }
}
impl Drop for Workspace {
    fn drop(&mut self) {
        if let Ok(entries) = fs::read_dir(WORKSPACE) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    let _ = fs::remove_dir_all(entry.path());
                } else {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
    }
}

fn prepare_workspace(
    root: &Path,
    preparations: &[DependencyPreparationPlan],
) -> Result<
    (
        Workspace,
        super::snapshot::ManagedWorkspaceSnapshot,
        tempfile::TempDir,
    ),
    BackendError,
> {
    let _workspace = Workspace::acquire()?;
    let snapshot = super::snapshot::create(root, preparations)?;
    copy_tree(snapshot.checkout.path(), Path::new(WORKSPACE))?;
    let state = tempfile::tempdir().map_err(io_error)?;
    let cache = state.path().join("cache");
    copy_tree(Path::new(crate::preparation::CACHE_SEED_ROOT), &cache)?;
    let output = root.join(".ayni");
    if !fs::symlink_metadata(&output)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.file_type().is_symlink())
    {
        return Err(BackendError::environment(
            "mount a writable .ayni output directory inside the read-only source",
        ));
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(&output, Path::new(WORKSPACE).join(".ayni")).map_err(io_error)?;
    for output in crate::preparation::unique_outputs(preparations) {
        let destination = Path::new(WORKSPACE).join(&output.mount_path);
        if output.mode == PreparationOutputMode::Seeded {
            copy_tree(
                &Path::new(crate::preparation::SEED_ROOT)
                    .join(crate::preparation::output_key(&output)),
                &destination,
            )?;
        } else {
            fs::create_dir_all(destination).map_err(io_error)?;
        }
    }
    Ok((_workspace, snapshot, state))
}

pub fn launch(
    root: &Path,
    preparations: &[DependencyPreparationPlan],
    command: &[String],
    authorization: LaunchAuthorization,
    inputs: &[ReadOnlyInput],
    runtime: &prebuilt::RuntimeIdentity,
) -> Result<CapturedLaunch, BackendError> {
    let lock = crate::read_lock(root)?;
    validate_posture(root, &lock, authorization)?;
    let (_workspace, snapshot, state) = prepare_workspace(root, preparations)?;
    let cache = state.path().join("cache");
    let mut environment = base_environment(state.path());
    let environments = crate::preparation::managed_environments(
        &lock,
        preparations,
        &state.path().display().to_string(),
    )?;
    let environments = environments.replace("/home/ayni/.cache", &cache.display().to_string());
    materialize(preparations, &environments, &environment)?;
    // Re-read certificate identity immediately before the protected runner starts.
    if prebuilt::discover()?.as_ref() != Some(runtime) {
        return Err(BackendError::environment(
            "prebuilt runtime changed before execution",
        ));
    }
    environment.insert(
        "AYNI_MANAGED_PREBUILT_SOURCE".into(),
        root.display().to_string(),
    );
    environment.insert(
        "AYNI_MANAGED_PREBUILT_AUTHORIZATION".into(),
        serde_json::to_string(&authorization).map_err(io_error)?,
    );
    add_execution_context(
        &mut environment,
        &lock,
        runtime,
        snapshot.manifest.path(),
        environments,
        inputs,
    )?;
    let command = command
        .iter()
        .map(|argument| {
            inputs
                .iter()
                .find(|input| input.destination == *argument)
                .map_or_else(
                    || argument.clone(),
                    |input| input.source.display().to_string(),
                )
        })
        .collect::<Vec<_>>();
    let output = std::process::Command::new("/usr/local/bin/ayni")
        .args(command)
        .current_dir(WORKSPACE)
        .env_clear()
        .envs(environment)
        .output()
        .map_err(io_error)?;
    Ok(CapturedLaunch {
        code: output.status.code().unwrap_or(4),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn add_execution_context(
    environment: &mut BTreeMap<String, String>,
    lock: &EnvironmentLock,
    runtime: &prebuilt::RuntimeIdentity,
    manifest: &Path,
    environments: String,
    inputs: &[ReadOnlyInput],
) -> Result<(), BackendError> {
    environment.extend(BTreeMap::from([
        ("AYNI_MANAGED_TARGET_ENVIRONMENTS".into(), environments),
        (
            "AYNI_MANAGED_WORKSPACE_MANIFEST".into(),
            manifest.display().to_string(),
        ),
        ("AYNI_MANAGED_WORKSPACE_ROOT".into(), WORKSPACE.into()),
        (
            "AYNI_MANAGED_LOCK_FINGERPRINT".into(),
            lock.fingerprint().into(),
        ),
        (
            "AYNI_MANAGED_TOOL_VERSIONS".into(),
            serde_json::to_string(&super::locked_tool_versions(lock)).map_err(io_error)?,
        ),
        (
            "AYNI_MANAGED_PREBUILT_RUNTIME".into(),
            serde_json::to_string(runtime).map_err(io_error)?,
        ),
    ]));
    for input in inputs {
        if input.destination == "/opt/ayni/inputs/impact-plan.json" {
            environment.insert(
                "AYNI_MANAGED_IMPACT_HANDOFF".into(),
                input.source.display().to_string(),
            );
        }
    }
    Ok(())
}

fn base_environment(state: &Path) -> BTreeMap<String, String> {
    let cache = state.join("cache");
    BTreeMap::from([
        (
            "PATH".into(),
            "/opt/ayni/mise/shims:/usr/local/bin:/usr/bin:/bin".into(),
        ),
        ("HOME".into(), state.display().to_string()),
        ("XDG_CACHE_HOME".into(), cache.display().to_string()),
        (
            "XDG_STATE_HOME".into(),
            state.join("state").display().to_string(),
        ),
        ("MISE_DATA_DIR".into(), "/opt/ayni/mise".into()),
        ("MISE_CONFIG_FILE".into(), "/etc/ayni/mise.toml".into()),
        ("MISE_TRUSTED_CONFIG_PATHS".into(), "/etc/ayni".into()),
        (
            "MISE_CACHE_DIR".into(),
            cache.join("mise").display().to_string(),
        ),
        ("MISE_AUTO_INSTALL".into(), "0".into()),
        (
            "CARGO_HOME".into(),
            cache.join("cargo").display().to_string(),
        ),
        ("RUSTUP_HOME".into(), "/home/ayni/.rustup".into()),
        (
            "npm_config_cache".into(),
            cache.join("npm").display().to_string(),
        ),
        ("LANG".into(), "C.UTF-8".into()),
    ])
}

fn materialize(
    plans: &[DependencyPreparationPlan],
    environments: &str,
    base: &BTreeMap<String, String>,
) -> Result<(), BackendError> {
    let environments: BTreeMap<String, BTreeMap<String, String>> =
        serde_json::from_str(environments).map_err(io_error)?;
    let mut seen = Vec::new();
    for plan in plans {
        for command in &plan.materialization_commands {
            let mut environment = base.clone();
            environment.extend(environments[&crate::preparation::target_key(&plan.target)].clone());
            environment.extend(command.environment.iter().map(|(key, value)| {
                (
                    key.clone(),
                    value.replace("/home/ayni/.cache", &base["XDG_CACHE_HOME"]),
                )
            }));
            let mut args = vec!["-i".into()];
            args.extend(
                environment
                    .iter()
                    .map(|(key, value)| format!("{key}={value}")),
            );
            args.push(command.program.clone());
            args.extend(command.args.clone());
            let cwd = Path::new(WORKSPACE).join(&command.cwd);
            if !seen.contains(&(cwd.clone(), args.clone())) {
                execute(&cwd, "/usr/bin/env", &args)?;
                seen.push((cwd, args));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_must_drop_privileges_and_all_capability_sets() {
        let valid = "Uid:\t10001 10001 10001 10001\nGid:\t10001 10001 10001 10001\nCapInh:\t0\nCapPrm:\t0\nCapEff:\t0\nCapBnd:\t0\nCapAmb:\t0\nNoNewPrivs:\t1\n";
        validate_process_status(valid).unwrap();
        for invalid in [
            valid.replace("CapBnd:\t0", "CapBnd:\t1"),
            valid.replace("NoNewPrivs:\t1", "NoNewPrivs:\t0"),
            valid.replace("10001", "0"),
            String::new(),
        ] {
            assert!(validate_process_status(&invalid).is_err());
        }
    }
}
