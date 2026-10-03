use crate::BackendError;
use crate::runtime::target_environment;
use ayni_core::{
    DependencyPreparationPlan, EnvironmentLock, PreparationOutput, PreparationOutputMode,
    sha256_fingerprint,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const INPUT_ROOT: &str = "/tmp/ayni/repository";
pub(crate) const SEED_ROOT: &str = "/opt/ayni/preparation";
const PREPARATION_IMPLEMENTATION_VERSION: &str = "14";

pub(crate) fn dockerfile_fragment(
    lock: &EnvironmentLock,
    plans: &[DependencyPreparationPlan],
) -> Result<String, BackendError> {
    if plans.is_empty() {
        return Ok(String::new());
    }
    let groups = crate::preparation_groups::groups(plans)?;
    let mut output = String::new();
    for group in &groups {
        output.push_str(&format!("FROM ayni-runtime AS preparation-{}\nCOPY --chown=10001:10001 groups/{} /tmp/ayni/repository\n", group.id, group.id));
        let mut commands = Vec::new();
        for plan in &group.plans {
            let target = lock
                .targets()
                .iter()
                .find(|target| target.target == plan.target)
                .ok_or_else(|| {
                    BackendError::environment("dependency preparation target is absent from lock")
                })?;
            let activation = target_environment(target)?;
            for command in &plan.commands {
                let instruction = preparation_run_instruction(command, &activation);
                if !commands.contains(&instruction) {
                    output.push_str(&instruction);
                    output.push('\n');
                    commands.push(instruction);
                }
            }
        }
        let paths = unique_outputs(&group.plans)
            .into_iter()
            .filter(|item| item.mode != PreparationOutputMode::Fresh)
            .map(|item| item.path)
            .collect::<Vec<_>>();
        output.push_str("RUN mkdir -p /tmp/ayni-seeds\n");
        if !paths.is_empty() {
            let mut directories = vec!["mkdir".to_owned(), "-p".into(), "--".into()];
            directories.extend(paths.iter().map(|path| docker_path(INPUT_ROOT, path)));
            output.push_str(&format!(
                "RUN {}\n",
                serde_json::to_string(&directories).unwrap()
            ));
        }
        let mut archive = vec![
            "tar".to_owned(),
            "--hard-dereference".into(),
            "-cf".into(),
            "/tmp/ayni-seeds/outputs.tar".into(),
            "-C".into(),
            INPUT_ROOT.into(),
        ];
        if paths.is_empty() {
            archive.extend(["--files-from".into(), "/dev/null".into()]);
        } else {
            archive.push("--".into());
            archive.extend(paths);
        }
        output.push_str(&format!(
            "RUN {}\n",
            serde_json::to_string(&archive).unwrap()
        ));
        output.push_str(
            "RUN tar --hard-dereference -cf /tmp/ayni-seeds/cache.tar -C /home/ayni/.cache .\n",
        );
    }
    output.push_str("FROM ayni-tools\n");
    for group in &groups {
        output.push_str(&format!(
            "COPY --from=preparation-{} /tmp/ayni-seeds/ {SEED_ROOT}/{}/\n",
            group.id, group.id
        ));
    }
    output.push_str(&format!("COPY preparation.json {SEED_ROOT}/plans.json\n"));
    Ok(output)
}

pub(crate) fn stage_inputs(
    repo_root: &Path,
    context_root: &Path,
    plans: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    fs::write(
        context_root.join("preparation.json"),
        serde_json::to_vec(plans).unwrap(),
    )
    .map_err(|error| BackendError::execution(format!("write preparation metadata: {error}")))?;
    let groups_root = context_root.join("groups");
    fs::create_dir(&groups_root).map_err(|error| {
        BackendError::execution(format!("failed to create preparation groups: {error}"))
    })?;
    for group in crate::preparation_groups::groups(plans)? {
        let destination_root = groups_root.join(&group.id);
        stage_workspace(repo_root, &destination_root, &group.plans)?;
    }
    Ok(())
}

pub(crate) fn stage_workspace(
    repo_root: &Path,
    destination_root: &Path,
    plans: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    fs::create_dir(destination_root).map_err(|error| {
        BackendError::execution(format!("failed to stage preparation workspace: {error}"))
    })?;
    stage_locked_inputs(repo_root, destination_root, plans)?;
    stage_scaffolds(destination_root, plans)
}

fn stage_locked_inputs(
    repo_root: &Path,
    destination_root: &Path,
    plans: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    let mut staged = BTreeMap::new();
    for plan in ordered_plans(plans) {
        for input in &plan.inputs {
            if let Some(previous) = staged.insert(input.path.clone(), input.digest.clone())
                && previous != input.digest
            {
                return Err(BackendError::environment(format!(
                    "dependency input {} has conflicting locked digests",
                    input.path
                )));
            }
            stage_locked_input(repo_root, destination_root, input)?;
        }
    }
    Ok(())
}

fn stage_locked_input(
    repo_root: &Path,
    destination_root: &Path,
    input: &ayni_core::PreparationInput,
) -> Result<(), BackendError> {
    let source = contained_file(repo_root, &input.path)?;
    let bytes = fs::read(&source).map_err(|error| {
        BackendError::environment(format!(
            "failed to read dependency input {}: {error}",
            input.path
        ))
    })?;
    let actual = sha256_fingerprint(&bytes);
    if actual != input.digest {
        return Err(BackendError::environment(format!(
            "dependency input {} changed; run `ayni env lock`",
            input.path
        )));
    }
    let destination = destination_root.join(&input.path);
    create_parent(&destination, "staged dependency input")?;
    if destination.exists() {
        return Ok(());
    }
    fs::write(&destination, bytes).map_err(|error| {
        BackendError::execution(format!(
            "failed to stage dependency input {}: {error}",
            input.path
        ))
    })
}

fn stage_scaffolds(
    destination_root: &Path,
    plans: &[DependencyPreparationPlan],
) -> Result<(), BackendError> {
    let mut generated = BTreeMap::new();
    for plan in ordered_plans(plans) {
        for scaffold in &plan.scaffolds {
            if let Some(previous) =
                generated.insert(scaffold.path.clone(), scaffold.content.clone())
                && previous != scaffold.content
            {
                return Err(BackendError::environment(format!(
                    "preparation scaffold {} has conflicting contents",
                    scaffold.path
                )));
            }
            stage_scaffold(destination_root, scaffold)?;
        }
    }
    Ok(())
}

fn stage_scaffold(
    destination_root: &Path,
    scaffold: &ayni_core::PreparationScaffold,
) -> Result<(), BackendError> {
    let destination = destination_root.join(&scaffold.path);
    if destination.exists() {
        return Ok(());
    }
    create_parent(&destination, "preparation scaffold")?;
    fs::write(&destination, scaffold.content.as_bytes()).map_err(|error| {
        BackendError::execution(format!(
            "failed to write preparation scaffold {}: {error}",
            scaffold.path
        ))
    })
}

fn create_parent(path: &Path, description: &str) -> Result<(), BackendError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    fs::create_dir_all(parent).map_err(|error| {
        BackendError::execution(format!("failed to create {description} directory: {error}"))
    })
}

pub(crate) fn preparation_digest(
    plans: &[DependencyPreparationPlan],
) -> Result<String, BackendError> {
    let plans = ordered_plans(plans);
    let bytes =
        serde_json::to_vec(&(PREPARATION_IMPLEMENTATION_VERSION, plans)).map_err(|error| {
            BackendError::execution(format!(
                "failed to serialize dependency preparation: {error}"
            ))
        })?;
    Ok(sha256_fingerprint(bytes))
}

pub(crate) fn unique_outputs(plans: &[DependencyPreparationPlan]) -> Vec<PreparationOutput> {
    let mut outputs = BTreeSet::new();
    for plan in plans {
        outputs.extend(plan.outputs.iter().cloned());
    }
    outputs.into_iter().collect()
}

fn ordered_plans(plans: &[DependencyPreparationPlan]) -> Vec<&DependencyPreparationPlan> {
    let mut plans = plans.iter().collect::<Vec<_>>();
    plans.sort_by(|left, right| left.target.cmp(&right.target));
    plans
}

fn docker_path(root: &str, relative: &str) -> String {
    if relative == "." {
        root.to_owned()
    } else {
        format!("{root}/{relative}")
    }
}

fn preparation_run_instruction(
    command: &ayni_core::PreparationCommand,
    activation: &[(String, String)],
) -> String {
    // Keep repository-derived paths out of Dockerfile instruction syntax. The
    // constant shell program receives the working directory as a JSON-encoded
    // positional argument, then executes the adapter command without reparsing
    // any of its data as shell source.
    let mut argv = vec![
        String::from("/bin/sh"),
        String::from("-c"),
        String::from("cd \"$1\" && shift && exec env \"$@\""),
        String::from("ayni-preparation"),
        docker_path(INPUT_ROOT, &command.cwd),
        String::from("env"),
    ];
    argv.extend(
        activation
            .iter()
            .map(|(name, value)| format!("{name}={value}")),
    );
    argv.extend(
        command
            .environment
            .iter()
            .map(|(name, value)| format!("{name}={value}")),
    );
    argv.extend([
        String::from("/bin/sh"),
        String::from("-eu"),
        String::from("-c"),
        String::from(
            "environment=\"$(/usr/local/bin/mise -C /etc/ayni env -s bash)\"; eval \"$environment\"; exec \"$@\"",
        ),
        String::from("ayni-preparation-activate"),
        command.program.clone(),
    ]);
    argv.extend(command.args.clone());
    format!(
        "RUN {}",
        serde_json::to_string(&argv).expect("argv serialization")
    )
}

fn contained_file(repo_root: &Path, relative: &str) -> Result<PathBuf, BackendError> {
    let source = repo_root.join(relative);
    let canonical = source.canonicalize().map_err(|error| {
        BackendError::environment(format!(
            "failed to inspect dependency input {relative}: {error}"
        ))
    })?;
    if canonical.starts_with(repo_root) && canonical.is_file() {
        Ok(canonical)
    } else {
        Err(BackendError::environment(format!(
            "dependency input escapes the repository or is not a file: {relative}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ayni_core::{Language, PreparationCommand, PreparationInput, TargetIdentity};

    fn plan(root: &str, commands: &[&str]) -> DependencyPreparationPlan {
        DependencyPreparationPlan {
            target: TargetIdentity::new(Language::Rust, root).expect("target"),
            inputs: vec![PreparationInput {
                path: format!("{root}/Cargo.lock"),
                digest: format!("sha256:{}", "0".repeat(64)),
                owner_root: root.into(),
            }],
            commands: commands
                .iter()
                .map(|program| PreparationCommand {
                    program: (*program).into(),
                    args: Vec::new(),
                    cwd: root.into(),
                    environment: BTreeMap::new(),
                })
                .collect(),
            scaffolds: Vec::new(),
            materialization_commands: Vec::new(),
            outputs: Vec::new(),
            execution_environment: BTreeMap::new(),
        }
    }

    #[test]
    fn preparation_digest_orders_targets_but_preserves_command_semantics() {
        let first = plan("one", &["cargo", "rustc"]);
        let second = plan("two", &["cargo"]);
        assert_eq!(
            preparation_digest(&[first.clone(), second.clone()]).expect("digest"),
            preparation_digest(&[second, first.clone()]).expect("digest")
        );
        assert_ne!(
            preparation_digest(std::slice::from_ref(&first)).expect("digest"),
            preparation_digest(&[plan("one", &["rustc", "cargo"])]).expect("digest")
        );
        assert_ne!(
            preparation_digest(std::slice::from_ref(&first)).expect("digest"),
            preparation_digest(&[plan("one", &["cargo", "rustc", "cargo"])]).expect("digest")
        );
    }

    #[test]
    fn preparation_run_encodes_repository_paths_as_json_data() {
        let cwd = "packages/space dir/\tcontrol\r\nRUN touch /tmp/injected";
        let command = PreparationCommand {
            program: String::from("cargo"),
            args: vec![String::from("fetch")],
            cwd: cwd.into(),
            environment: BTreeMap::new(),
        };

        let instruction = preparation_run_instruction(&command, &[]);

        assert!(!instruction.contains(['\n', '\r', '\t']));
        let argv = serde_json::from_str::<Vec<String>>(
            instruction.strip_prefix("RUN ").expect("RUN instruction"),
        )
        .expect("JSON-form RUN");
        assert_eq!(argv[0], "/bin/sh");
        assert_eq!(argv[2], "cd \"$1\" && shift && exec env \"$@\"");
        assert_eq!(argv[4], docker_path(INPUT_ROOT, cwd));
        assert_eq!(
            &argv[5..],
            [
                "env",
                "/bin/sh",
                "-eu",
                "-c",
                "environment=\"$(/usr/local/bin/mise -C /etc/ayni env -s bash)\"; eval \"$environment\"; exec \"$@\"",
                "ayni-preparation-activate",
                "cargo",
                "fetch",
            ]
        );
    }
}
