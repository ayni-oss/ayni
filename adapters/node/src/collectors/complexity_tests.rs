use super::*;
use ayni_core::{
    AyniPolicy, CompletionScope, ExecutionResolution, RunArtifact, RunArtifactMetadata,
    RunCompletion, Scope, VerificationTarget,
};
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn same_line_functions_keep_distinct_serializable_findings() {
    let directory = tempfile::tempdir().expect("fixture directory");
    let root = directory.path();
    let report = serde_json::json!([{
        "filePath":root.join("src/inline.ts"),
        "messages":[
            {"ruleId":"complexity","message":"Arrow function has a complexity of 3.","line":1,"column":12,"nodeType":"ArrowFunctionExpression"},
            {"ruleId":"complexity","message":"Arrow function has a complexity of 4.","line":1,"column":72,"nodeType":"ArrowFunctionExpression"}
        ]
    }]);
    fs::write(root.join("report.json"), report.to_string()).expect("report");
    let runner = root.join("runner");
    fs::write(
        &runner,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > args.txt\ncat report.json\n",
    )
    .expect("runner");
    fs::set_permissions(&runner, fs::Permissions::from_mode(0o755)).expect("permissions");
    let policy: AyniPolicy =
        toml::from_str("[node.complexity]\nfn_cyclomatic={warn=2,fail=5}").expect("policy");
    let context = RunContext {
        repo_root: root.to_path_buf(),
        target_root: root.to_path_buf(),
        workdir: root.to_path_buf(),
        policy: policy.into(),
        scope: Scope {
            workspace_root: root.to_string_lossy().into_owned(),
            path: Some(".".into()),
            ..Scope::default()
        },
        execution: ExecutionResolution::direct(
            runner.to_string_lossy(),
            root.to_path_buf(),
            "fixture",
            100,
        ),
        cancellation: Default::default(),
        debug: false,
    };
    let row = collect(&context).expect("complexity evidence");
    let args = fs::read_to_string(root.join("args.txt")).expect("arguments");
    assert!(args.contains(
        "--ignore-pattern\n**/{node_modules,.git,.ayni,.svelte-kit,build,dist,coverage}/**"
    ));
    assert!(row.pass);
    let mut findings = row
        .offenders
        .clone()
        .into_findings(Language::Node, &row.scope.workspace_root, |_| {
            VerificationTarget {
                file: Some("src/inline.ts".into()),
                ..VerificationTarget::default()
            }
        })
        .expect("findings");
    assert_eq!(findings.ids().len(), 2);
    assert_ne!(findings.ids()[0], findings.ids()[1]);
    findings
        .render_commands(|_| {
            Ok("ayni verify complexity --language node --file src/inline.ts".into())
        })
        .expect("commands");
    let mut artifact = RunArtifact::new(
        RunArtifactMetadata::default(),
        RunCompletion::complete(CompletionScope::Requested, 1),
        vec![row],
    )
    .expect("artifact");
    artifact.findings = vec![findings];
    let bytes = serde_json::to_vec(&artifact).expect("serialize complete artifact");
    let decoded: RunArtifact = serde_json::from_slice(&bytes).expect("decode artifact");
    assert_eq!(decoded, artifact);
}
