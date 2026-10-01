//! CLI policy orchestration for the portable environment backend.
use crate::application::EnvShowOperation;
use crate::application::OutputFormat;
use crate::application_error::ApplicationError;
use std::path::Path;

pub(crate) fn verify(
    root: &Path,
) -> Result<Option<ayni_environment::prebuilt::RuntimeIdentity>, ApplicationError> {
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
    Ok(Some(runtime))
}

pub(crate) fn validate_worker() -> Result<(), ApplicationError> {
    let Some(expected) = std::env::var_os("AYNI_MANAGED_PREBUILT_RUNTIME") else {
        return Ok(());
    };
    let expected: ayni_environment::prebuilt::RuntimeIdentity =
        serde_json::from_str(&expected.to_string_lossy()).map_err(|error| {
            ApplicationError::environment(format!("invalid runtime execution context: {error}"))
        })?;
    let root = std::env::current_dir()
        .map_err(|error| ApplicationError::environment(error.to_string()))?;
    let source = std::env::var("AYNI_MANAGED_PREBUILT_SOURCE")
        .map_err(|_| ApplicationError::environment("missing portable source context"))?;
    let authorization = std::env::var("AYNI_MANAGED_PREBUILT_AUTHORIZATION")
        .map_err(|_| ApplicationError::environment("missing portable authorization context"))?;
    let authorization = serde_json::from_str(&authorization).map_err(|error| {
        ApplicationError::environment(format!("invalid portable authorization context: {error}"))
    })?;
    ayni_environment::validate_prebuilt_posture(
        Path::new(&source),
        &ayni_environment::read_lock(&root)?,
        authorization,
    )?;
    crate::environment_backend::validate_quality_source(&root, &crate::build_registry())?;
    if verify(&root)?.as_ref() != Some(&expected) {
        return Err(ApplicationError::environment(
            "prebuilt runtime changed before execution",
        ));
    }
    Ok(())
}

pub(crate) fn resolve_source_config(operation: &mut crate::application::Operation) {
    use crate::application::{ExecutionMode, Operation};
    let Some(root) = std::env::var_os("AYNI_SOURCE_ROOT").filter(|value| !value.is_empty()) else {
        return;
    };
    let (config, mode) = match operation {
        Operation::Check(value) => (&mut value.config, value.execution_mode),
        Operation::Verify(value) => (&mut value.config, value.execution_mode),
        Operation::ImpactRun(value) => (&mut value.config, value.execution_mode),
        _ => return,
    };
    if mode == ExecutionMode::Managed
        && std::fs::symlink_metadata(ayni_environment::prebuilt::METADATA_PATH).is_ok()
        && config.is_relative()
    {
        *config = Path::new(&root).join(&*config);
    }
}
