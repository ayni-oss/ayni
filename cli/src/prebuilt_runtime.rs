//! Resolve a verified Ayni environment for the current process.
//!
//! Quality commands never launch or re-execute through another container. A
//! runtime marker is only a discovery signal; when it is present it must verify
//! against the attached checkout before collectors may run.
use crate::application::{EnvShowOperation, OutputFormat};
use crate::application_error::ApplicationError;
use ayni_core::{ArtifactToolVersion, Language};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Debug)]
pub(crate) struct Context {
    pub runtime: ayni_environment::prebuilt::RuntimeIdentity,
    pub lock_fingerprint: String,
    pub tool_versions: Vec<ArtifactToolVersion>,
    pub targets: BTreeMap<String, BTreeMap<String, String>>,
}

static CONTEXT: OnceLock<Mutex<Option<Context>>> = OnceLock::new();

fn context_slot() -> &'static Mutex<Option<Context>> {
    CONTEXT.get_or_init(|| Mutex::new(None))
}

pub(crate) fn current() -> Option<Context> {
    context_slot().lock().expect("runtime context lock").clone()
}

pub(crate) fn activate(config: &Path) -> Result<(), ApplicationError> {
    let root = config
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .canonicalize()
        .map_err(|error| {
            ApplicationError::input(format!(
                "failed to resolve checkout root for {}: {error}",
                config.display()
            ))
        })?;
    let Some((runtime, lock)) = verify_with_lock(&root)? else {
        *context_slot().lock().expect("runtime context lock") = None;
        return Ok(());
    };
    let targets = lock
        .targets()
        .iter()
        .map(|target| {
            let key = format!("{}:{}", target.target.language, target.target.root);
            let activation = ayni_environment::target_environment(target)
                .map_err(|error| ApplicationError::environment(error.message))?
                .into_iter()
                .collect();
            Ok((key, activation))
        })
        .collect::<Result<_, ApplicationError>>()?;
    *context_slot().lock().expect("runtime context lock") = Some(Context {
        runtime,
        lock_fingerprint: lock.fingerprint().into(),
        tool_versions: ayni_environment::locked_tool_versions(&lock),
        targets,
    });
    Ok(())
}

fn verify_with_lock(
    root: &Path,
) -> Result<
    Option<(
        ayni_environment::prebuilt::RuntimeIdentity,
        ayni_core::EnvironmentLock,
    )>,
    ApplicationError,
> {
    let Some(runtime) = ayni_environment::prebuilt::discover()? else {
        return Ok(None);
    };
    let lock = ayni_environment::read_lock(root)?;
    let context = EnvShowOperation {
        config: lock.repository().contract_path.clone().into(),
        repo_root: root.into(),
        output: OutputFormat::Json,
    };
    let (_, _, _, policy) = crate::environment::load_context(&context)?;
    let trust = policy
        .environment_certificate_trust_policy()
        .map_err(ApplicationError::environment)?;
    ayni_environment::prebuilt::verify(&runtime, &lock, &trust)?;
    Ok(Some((runtime, lock)))
}

pub(crate) fn exec_prepared(command: Vec<String>) -> std::process::ExitCode {
    match prepare_current() {
        Ok(environment) => {
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new(&command[0])
                .args(&command[1..])
                .envs(environment)
                .exec();
            crate::application_error::render_error(ApplicationError::execution(error.to_string()))
        }
        Err(error) => crate::application_error::render_error(error),
    }
}

fn prepare_current() -> Result<BTreeMap<String, String>, ApplicationError> {
    let root = std::env::current_dir().map_err(|e| ApplicationError::input(e.to_string()))?;
    if !root.join(".ayni.lock").is_file() {
        return Ok(BTreeMap::new());
    }
    let Some((_, lock)) = verify_with_lock(&root)? else {
        return Ok(BTreeMap::new());
    };
    Ok(ayni_environment::materialize::prepare(
        &root,
        lock.fingerprint(),
    )?)
}

pub(crate) fn target_environment(
    language: Language,
    root: &str,
) -> Option<BTreeMap<String, String>> {
    current()?
        .targets
        .get(&format!("{language}:{root}"))
        .cloned()
}
