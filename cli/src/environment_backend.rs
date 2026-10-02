//! Explicit environment lifecycle commands.
//!
//! The environment backend builds and inspects OCI images. Quality commands do
//! not pass through this module: they run directly in the caller's workspace.
use crate::application::{
    EnvBuildOperation, EnvPruneOperation, EnvShowOperation, EnvStorageOperation, OutputFormat,
    RepositoryOperation,
};
use ayni_core::{
    AdapterRegistry, DependencyPreparationPlan, DependencyPreparationRequest, EnvironmentPlan,
};
use std::path::Path;
use std::process::ExitCode;

fn render_error(error: ayni_environment::BackendError) -> ExitCode {
    crate::application_error::render_error(error.into())
}

fn result(result: Result<String, ayni_environment::BackendError>) -> ExitCode {
    match result {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => render_error(error),
    }
}

pub(crate) fn doctor(operation: RepositoryOperation, registry: &AdapterRegistry) -> ExitCode {
    result((|| {
        let (root, plan) = current_plan(&operation.repo_root, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::doctor_prepared(&root, &preparations)
    })())
}

pub(crate) fn build(operation: EnvBuildOperation, registry: &AdapterRegistry) -> ExitCode {
    result((|| {
        let (root, plan) = current_plan(&operation.repo_root, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        let image = ayni_environment::build_prepared_with_cache(
            &root,
            &preparations,
            operation.executor_image.as_deref(),
            &ayni_environment::BuildCache {
                from: operation.cache_from,
                to: operation.cache_to,
            },
        )?;
        if let Some(tag) = operation.tag {
            if tag.is_empty() || tag.contains(char::is_whitespace) {
                return Err(ayni_environment::BackendError::input(
                    "--tag must be a non-empty OCI tag",
                ));
            }
            let source = image
                .strip_prefix("built ")
                .or_else(|| image.strip_prefix("current "))
                .ok_or_else(|| {
                    ayni_environment::BackendError::execution(
                        "environment build did not report its image tag",
                    )
                })?;
            let engine = match ayni_environment::detect_engine()? {
                ayni_environment::Engine::Docker => "docker",
                ayni_environment::Engine::Podman => "podman",
            };
            let status = std::process::Command::new(engine)
                .args(["image", "tag", source, &tag])
                .status()
                .map_err(|error| {
                    ayni_environment::BackendError::execution(format!(
                        "failed to apply image tag: {error}"
                    ))
                })?;
            if !status.success() {
                return Err(ayni_environment::BackendError::execution(
                    "container engine failed to apply --tag",
                ));
            }
            return Ok(format!("{image}\ntagged {tag}"));
        }
        Ok(image)
    })())
}

pub(crate) fn storage(operation: EnvStorageOperation, registry: &AdapterRegistry) -> ExitCode {
    let report = (|| {
        let (root, plan) = current_plan(&operation.repo_root, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::storage_report_prepared(&root, &preparations)
    })();
    match report {
        Ok(report) => match operation.output {
            OutputFormat::Human => {
                println!("Ayni environment storage ({})", report.engine);
                println!("Expected image: {}", report.expected_image_tag);
                println!("Current image present: {}", report.current_image_present);
                println!("Images: {}", report.images.len());
                ExitCode::SUCCESS
            }
            OutputFormat::Json => match serde_json::to_string_pretty(&report) {
                Ok(value) => {
                    println!("{value}");
                    ExitCode::SUCCESS
                }
                Err(error) => crate::application_error::render_error(
                    crate::application_error::ApplicationError::execution(error.to_string()),
                ),
            },
            OutputFormat::Markdown => unreachable!("env storage does not support markdown"),
        },
        Err(error) => render_error(error),
    }
}

pub(crate) fn prune(operation: EnvPruneOperation, registry: &AdapterRegistry) -> ExitCode {
    let report = (|| {
        let (root, plan) = current_plan(&operation.repo_root, registry)?;
        let preparations = dependency_preparations(&root, registry, &plan)?;
        ayni_environment::prune_storage_prepared(
            &root,
            &preparations,
            operation.apply,
            operation.images,
            operation.current,
        )
    })();
    match report {
        Ok(report) => match operation.output {
            OutputFormat::Human => {
                println!(
                    "Ayni environment prune: {} image(s), {} state generation(s) removed",
                    report.removed_images.len(),
                    report.removed_state_generations.len()
                );
                if report.complete() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(4)
                }
            }
            OutputFormat::Json => match serde_json::to_string_pretty(&report) {
                Ok(value) => {
                    println!("{value}");
                    if report.complete() {
                        ExitCode::SUCCESS
                    } else {
                        ExitCode::from(4)
                    }
                }
                Err(error) => crate::application_error::render_error(
                    crate::application_error::ApplicationError::execution(error.to_string()),
                ),
            },
            OutputFormat::Markdown => unreachable!("env prune does not support markdown"),
        },
        Err(error) => render_error(error),
    }
}

fn current_plan(
    repo_root: &Path,
    registry: &AdapterRegistry,
) -> Result<(std::path::PathBuf, EnvironmentPlan), ayni_environment::BackendError> {
    let root = repo_root.canonicalize().map_err(|error| {
        ayni_environment::BackendError::input(format!(
            "failed to establish repository root {}: {error}",
            repo_root.display()
        ))
    })?;
    let lock = ayni_environment::read_lock(&root)?;
    let operation = EnvShowOperation {
        config: lock.repository().contract_path.clone().into(),
        repo_root: root.clone(),
        output: OutputFormat::Json,
    };
    let plan = crate::environment::build_plan(&operation, registry).map_err(|error| {
        ayni_environment::BackendError::environment(format!(
            "environment lock is stale or unsupported: {}; run `ayni env lock`",
            error.message
        ))
    })?;
    if !plan.conflicts().is_empty() || !ayni_environment::plan_matches_lock(&plan, &lock) {
        return Err(ayni_environment::BackendError::environment(
            "environment lock is stale because discovered requirements changed; run `ayni env lock`",
        ));
    }
    Ok((root, plan))
}

fn dependency_preparations(
    repo_root: &Path,
    registry: &AdapterRegistry,
    plan: &EnvironmentPlan,
) -> Result<Vec<DependencyPreparationPlan>, ayni_environment::BackendError> {
    plan.targets()
        .iter()
        .map(|target| {
            let adapter = registry
                .adapters()
                .iter()
                .find(|adapter| adapter.language() == target.target.language)
                .ok_or_else(|| {
                    ayni_environment::BackendError::environment(format!(
                        "no adapter can prepare dependencies for {}",
                        target.target.language
                    ))
                })?;
            let request =
                DependencyPreparationRequest::new(repo_root.to_path_buf(), target.clone())
                    .map_err(|error| {
                        ayni_environment::BackendError::environment(error.to_string())
                    })?;
            adapter
                .prepare_dependencies(&request)
                .map_err(|error| ayni_environment::BackendError::environment(error.to_string()))
        })
        .collect()
}
