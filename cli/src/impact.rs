use crate::analysis::{
    build_analyze_targets, enabled_signal_kinds, invalidate_artifact_at, persist_artifact_at,
    signal_kind_slug, verified_environment_active, workspace_root_from_config_path,
};
use crate::application::{ImpactOperation, OutputFormat};
use crate::build_registry;
use crate::policy::load_from_path;
use crate::ui::cancellation::SignalCancellation;
use ayni_adapters_common::exec::run_command_structured_cancellable;
use ayni_adapters_common::paths::validate_configured_root_containment;
use ayni_core::{
    AdapterRegistry, AyniPolicy, CancellationToken, ChangedPath, Findings, ImpactArtifact,
    ImpactConfidence, ImpactExecutionIssue, ImpactIdentity, ImpactIdentityKind, ImpactPlan,
    ImpactReason, ImpactReasonKind, ImpactRequest, ImpactUncertainty, ImpactUncertaintyKind,
    RunOutcome, SelectedCheck, SignalRow, VerificationSelection,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

const IMPACT_ARTIFACT: &str = ".ayni/impact/last/impact.json";
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

mod git;
mod render;
use git::{GitSnapshot, git_snapshot};
use render::{emit_artifact, emit_plan, execution_mode_name};

type Error = crate::application_error::ApplicationError;

pub(crate) fn show(operation: ImpactOperation) -> ExitCode {
    let registry = build_registry();
    let cancellation = match SignalCancellation::install() {
        Ok(cancellation) => cancellation,
        Err(error) => return fail(Error::execution(error)),
    };
    match prepare_plan(&operation, &registry, &cancellation.token()) {
        Ok((_, _, plan, _)) => emit_plan(&plan, &operation)
            .map(|()| ExitCode::SUCCESS)
            .unwrap_or_else(fail),
        Err(error) => fail(error),
    }
}

pub(crate) fn run(operation: ImpactOperation) -> ExitCode {
    if let Err(error) = invalidate_run_artifact(&operation) {
        return fail(Error::execution(error));
    }
    let registry = build_registry();
    let cancellation = match SignalCancellation::install() {
        Ok(cancellation) => cancellation,
        Err(error) => return fail(Error::execution(error)),
    };
    match run_inner(&operation, &registry, &cancellation.token()) {
        Ok(RunOutcome::Passed) => ExitCode::SUCCESS,
        Ok(RunOutcome::QualityFailed) => ExitCode::from(1),
        Ok(RunOutcome::ExecutionIncomplete) => ExitCode::from(4),
        Err(error) => fail(error),
    }
}

pub(crate) fn invalidate_run_artifact(operation: &ImpactOperation) -> Result<(), String> {
    invalidate_artifact_at(
        &workspace_root_from_config_path(&operation.config)?,
        IMPACT_ARTIFACT,
    )
}

fn fail(error: Error) -> ExitCode {
    crate::application_error::render_error(error)
}

pub(super) fn ensure_not_cancelled(
    cancellation: &CancellationToken,
    operation: &str,
) -> Result<(), Error> {
    if cancellation.is_cancelled() {
        Err(Error::execution(format!("{operation} aborted by Ctrl-C")))
    } else {
        Ok(())
    }
}

fn prepare_plan(
    operation: &ImpactOperation,
    registry: &AdapterRegistry,
    cancellation: &CancellationToken,
) -> Result<(PathBuf, AyniPolicy, ImpactPlan, GitSnapshot), Error> {
    let workspace_root =
        workspace_root_from_config_path(&operation.config).map_err(Error::input)?;
    ensure_impact_artifact_ignored(&workspace_root, cancellation)?;
    let config = operation
        .config
        .canonicalize()
        .map_err(|error| Error::input(format!("failed to resolve impact contract: {error}")))?;
    if !config.starts_with(&workspace_root) {
        return Err(Error::input("impact contract escapes the repository root"));
    }
    let policy = load_from_path(&config).map_err(Error::input)?;
    validate_configured_root_containment(&workspace_root, &policy).map_err(Error::input)?;
    let snapshot = git_snapshot(&workspace_root, &operation.base, cancellation)?;
    let config_path = config
        .strip_prefix(&workspace_root)
        .map_err(|_| Error::input("impact contract escapes the repository root"))?
        .to_string_lossy()
        .replace('\\', "/");
    let plan = plan_changes(&workspace_root, &policy, &snapshot, &config_path, registry)?;
    Ok((workspace_root, policy, plan, snapshot))
}

fn plan_changes(
    workspace_root: &Path,
    policy: &AyniPolicy,
    snapshot: &GitSnapshot,
    config_path: &str,
    registry: &AdapterRegistry,
) -> Result<ImpactPlan, Error> {
    let signals = enabled_signal_kinds(policy)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let broad = snapshot
        .changes
        .iter()
        .any(|change| change_touches(change, config_path) || change_touches(change, ".ayni.lock"));
    let mut selected_checks = Vec::new();
    let mut uncertainties = Vec::new();
    for language in policy.enabled_languages().map_err(Error::input)? {
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == language)
            .ok_or_else(|| Error::execution(format!("{language} adapter is unavailable")))?;
        for root in policy.roots_for(language) {
            if broad {
                for signal in &signals {
                    selected_checks.push(SelectedCheck::root(
                        language,
                        root.clone(),
                        *signal,
                        ImpactReason {
                            kind: ImpactReasonKind::EnvironmentChanged,
                            detail: String::from("configuration or environment lock changed"),
                        },
                        ImpactConfidence::Certain,
                    ));
                }
                continue;
            }
            let request = ImpactRequest::new(
                workspace_root.to_path_buf(),
                language,
                root.clone(),
                snapshot.changes.clone(),
                signals.iter().copied(),
            )
            .map_err(|error| Error::input(error.to_string()))?;
            match adapter.analyze_impact(&request) {
                Ok(contribution) => {
                    selected_checks.extend(contribution.selected_checks);
                    uncertainties.extend(contribution.uncertainties);
                }
                Err(error) => {
                    uncertainties.push(ImpactUncertainty {
                        kind: ImpactUncertaintyKind::Unsupported,
                        detail: format!(
                            "{language}:{root} impact mapping broadened: {}",
                            error.message
                        ),
                    });
                    for signal in &signals {
                        selected_checks.push(SelectedCheck::root(
                            language,
                            root.clone(),
                            *signal,
                            ImpactReason {
                                kind: ImpactReasonKind::UnsupportedCapability,
                                detail: String::from(
                                    "unsupported impact mapping requires configured-root execution",
                                ),
                            },
                            ImpactConfidence::Unknown,
                        ));
                    }
                }
            }
        }
    }
    let mut plan = ImpactPlan {
        base: ImpactIdentity {
            kind: ImpactIdentityKind::Revision,
            revision: snapshot.base_commit.clone(),
            requested: Some(snapshot.requested_base.clone()),
            fingerprint: None,
        },
        candidate: ImpactIdentity {
            kind: ImpactIdentityKind::WorkingTree,
            revision: snapshot.head_commit.clone(),
            requested: None,
            fingerprint: Some(snapshot.fingerprint.clone()),
        },
        changes: snapshot.changes.clone(),
        selected_checks,
        uncertainties,
        repository_completion_required: true,
    };
    plan.normalize();
    plan.validate()
        .map_err(|error| Error::input(error.to_string()))?;
    Ok(plan)
}

fn change_touches(change: &ChangedPath, path: &str) -> bool {
    change.path == path || change.previous_path.as_deref() == Some(path)
}

fn ensure_impact_artifact_ignored(
    workspace_root: &Path,
    cancellation: &CancellationToken,
) -> Result<(), Error> {
    let args = vec![
        "check-ignore".into(),
        "-q".into(),
        "--".into(),
        IMPACT_ARTIFACT.into(),
    ];
    let output =
        run_command_structured_cancellable(workspace_root, "git", &args, GIT_TIMEOUT, cancellation)
            .map_err(|error| Error::execution(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else if output.status.code() == Some(1) {
        Err(Error::input(
            ".ayni/ must be ignored before impact planning can persist evidence",
        ))
    } else {
        Err(Error::input(format!(
            "git check-ignore failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn run_inner(
    operation: &ImpactOperation,
    registry: &AdapterRegistry,
    cancellation: &CancellationToken,
) -> Result<RunOutcome, Error> {
    let (workspace_root, policy, plan, before) = prepare_plan(operation, registry, cancellation)?;
    let mut collected = execute_checks(
        &workspace_root,
        &policy,
        &plan,
        registry,
        operation,
        cancellation,
    )?;
    let findings = materialize_findings(&collected.rows, registry, &operation.config)?;
    let (_, _, after_plan, after) = prepare_plan(operation, registry, cancellation)?;
    if after != before || after_plan != plan {
        collected.issues.push(ImpactExecutionIssue {
            check: None,
            message: String::from(
                "impact candidate changed during execution; rerun against a stable checkout",
            ),
        });
    }
    let generated_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|error| Error::execution(format!("failed to format timestamp: {error}")))?;
    let artifact = ImpactArtifact::new(
        generated_at,
        execution_mode_name(),
        plan,
        collected.issues,
        collected.rows,
        findings,
    );
    let serialized = serde_json::to_string_pretty(&artifact)
        .map(|value| format!("{value}\n"))
        .map_err(|error| {
            Error::execution(format!("failed to serialize impact artifact: {error}"))
        })?;
    persist_artifact_at(&workspace_root, IMPACT_ARTIFACT, &serialized).map_err(Error::execution)?;
    emit_artifact(&artifact, &serialized, operation.output)?;
    Ok(artifact.outcome())
}

struct CollectedImpact {
    rows: Vec<SignalRow>,
    issues: Vec<ImpactExecutionIssue>,
}

fn execute_checks(
    workspace_root: &Path,
    policy: &AyniPolicy,
    plan: &ImpactPlan,
    registry: &AdapterRegistry,
    operation: &ImpactOperation,
    cancellation: &CancellationToken,
) -> Result<CollectedImpact, Error> {
    let planning = build_analyze_targets(
        workspace_root,
        policy,
        None,
        None,
        None,
        operation.debug,
        registry,
    )
    .map_err(Error::input)?;
    let targets = planning
        .targets
        .iter()
        .map(|target| ((target.language, target.root.clone()), target))
        .collect::<BTreeMap<_, _>>();
    let mut collected = CollectedImpact {
        rows: Vec::new(),
        issues: Vec::new(),
    };
    for check in &plan.selected_checks {
        if cancellation.is_cancelled() {
            return Err(Error::execution("impact execution aborted by Ctrl-C"));
        }
        let Some(base_target) = targets.get(&(check.language, check.configured_root.clone()))
        else {
            collected.issues.push(ImpactExecutionIssue {
                check: Some(check.clone()),
                message: String::from("configured impact target was not detected or resolved"),
            });
            continue;
        };
        let adapter = registry
            .adapters()
            .iter()
            .find(|adapter| adapter.language() == check.language)
            .ok_or_else(|| Error::execution(format!("{} adapter unavailable", check.language)))?;
        let mut target = (*base_target).clone();
        target.run_context.cancellation = cancellation.clone();
        target.run_context.scope.package = check.package.clone();
        target.run_context.scope.file = check.file.clone();
        let selection = VerificationSelection {
            file: check.file.clone(),
            package: check.package.clone(),
            name: None,
        };
        let row = match adapter.collect_verification(
            check.signal,
            &target.run_context,
            &selection,
            &mut |line| {
                eprintln!(
                    "[impact {}:{}] {} {line}",
                    check.language,
                    check.configured_root,
                    signal_kind_slug(check.signal)
                );
            },
        ) {
            Ok(row) => row,
            Err(error) => {
                collected.issues.push(ImpactExecutionIssue {
                    check: Some(check.clone()),
                    message: format!("collection incomplete: {error}"),
                });
                continue;
            }
        };
        if let Some(failure) = row.result.command_failure() {
            collected.issues.push(ImpactExecutionIssue {
                check: Some(check.clone()),
                message: format!(
                    "{} failed to execute: {}",
                    signal_kind_slug(check.signal),
                    failure.message
                ),
            });
        }
        collected.rows.push(row);
    }
    Ok(collected)
}

fn materialize_findings(
    rows: &[SignalRow],
    registry: &AdapterRegistry,
    config_path: &Path,
) -> Result<Vec<Findings>, Error> {
    rows.iter()
        .map(|row| {
            let adapter = registry
                .adapters()
                .iter()
                .find(|adapter| adapter.language() == row.language)
                .ok_or_else(|| Error::execution(format!("{} adapter unavailable", row.language)))?;
            let mut findings = adapter
                .findings_for(row, &row.scope.workspace_root)
                .map_err(|error| {
                    Error::execution(format!("failed to map impact findings: {error}"))
                })?;
            let root = row.scope.path.as_deref().unwrap_or(".");
            findings
                .render_commands(|target| {
                    adapter
                        .verification_selector_support(row.kind)
                        .validate_target(row.kind, target)?;
                    Ok(crate::verification_command::render_verification_command(
                        &config_path.to_string_lossy(),
                        row.kind,
                        row.language,
                        root,
                        target,
                    ))
                })
                .map_err(|error| Error::execution(error.to_string()))?;
            Ok(findings)
        })
        .collect()
}
