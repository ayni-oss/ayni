use super::*;
use serde::Serialize;

#[derive(Serialize)]
struct PlanEnvelope<'a> {
    schema_version: &'static str,
    execution_mode: &'static str,
    plan: &'a ImpactPlan,
}

pub(super) fn emit_plan(plan: &ImpactPlan, operation: &ImpactOperation) -> Result<(), Error> {
    match operation.output {
        OutputFormat::Json => {
            let value = serde_json::to_string_pretty(&PlanEnvelope {
                schema_version: ayni_core::IMPACT_SCHEMA_VERSION,
                execution_mode: execution_mode_name(),
                plan,
            })
            .map_err(|error| {
                Error::execution(format!("failed to serialize impact plan: {error}"))
            })?;
            println!("{value}");
        }
        OutputFormat::Markdown => print_plan_markdown(plan),
        OutputFormat::Human => print_plan_human(plan),
    }
    Ok(())
}

pub(super) fn emit_artifact(
    artifact: &ImpactArtifact,
    serialized: &str,
    output: OutputFormat,
) -> Result<(), Error> {
    match output {
        OutputFormat::Json => print!("{serialized}"),
        OutputFormat::Markdown => print_plan_markdown(&artifact.plan),
        OutputFormat::Human => {
            print_plan_human(&artifact.plan);
            println!("execution: {:?}", artifact.execution.state);
        }
    }
    Ok(())
}

fn print_plan_human(plan: &ImpactPlan) {
    println!("ayni impact plan");
    println!(
        "base: {} -> {}",
        plan.base.requested.as_deref().unwrap_or("<unknown>"),
        plan.base.revision
    );
    println!("environment: {}", execution_mode_name());
    println!("changes: {}", plan.changes.len());
    println!("selected checks: {}", plan.selected_checks.len());
    for check in &plan.selected_checks {
        println!(
            "  {}:{} {}",
            check.language,
            check.configured_root,
            signal_kind_slug(check.signal)
        );
    }
    println!("impact evidence is not repository completion; run `ayni check`");
}

fn print_plan_markdown(plan: &ImpactPlan) {
    println!("# Ayni impact plan\n");
    println!("- Base: `{}`", plan.base.revision);
    println!("- Environment: `{}`", execution_mode_name());
    println!("- Changed paths: {}", plan.changes.len());
    println!("- Selected checks: {}", plan.selected_checks.len());
}

pub(super) fn execution_mode_name() -> &'static str {
    if verified_environment_active() {
        "verified-environment"
    } else {
        "local"
    }
}
