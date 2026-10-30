use crate::application::{
    CheckOperation, ContractOperation, EnvCertifyOperation, EnvLockOperation, EnvPruneOperation,
    EnvShowOperation, EnvStorageOperation, ImpactOperation, InitOperation, Operation, OutputFormat,
    RepositoryOperation, ResultsCompareOperation, VerifyListOperation, VerifyOperation,
};
use ayni_core::{Language, SignalKind};
use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

const DEFAULT_CONFIG: &str = "./.ayni.toml";
const DEFAULT_RESULTS_ARTIFACT: &str = "./.ayni/last/signals.json";

#[derive(Parser, Debug)]
#[command(name = "ayni")]
#[command(
    version,
    about = "Reproducible environments, focused feedback, explicit completion evidence"
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Commands,
}

impl Cli {
    pub(crate) fn into_operation(self) -> Operation {
        self.command.into_operation()
    }
}

#[derive(Subcommand, Debug)]
enum Commands {
    #[command(hide = true, name = "__validate-seeds")]
    ValidateSeeds,
    #[command(hide = true, name = "__exec")]
    PreparedExec {
        #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Preview or write a minimal policy from adapter-owned project discovery.
    Init(InitOptions),
    /// Inspect and manage the repository code environment.
    Env {
        #[command(subcommand)]
        command: EnvCommands,
    },
    /// Preview signal-tool declarations, baseline drift, and blockers.
    Tools {
        #[command(subcommand)]
        command: ToolsCommands,
    },
    /// Inspect the repository quality contract.
    Contract {
        #[command(subcommand)]
        command: ContractCommands,
    },
    /// Run focused signals or list exact commands from saved evidence.
    Verify {
        #[command(subcommand)]
        command: VerifyCommands,
    },
    /// Explain or run the checks affected by an explicit change.
    Impact {
        #[command(subcommand)]
        command: ImpactCommands,
    },
    /// Run the complete repository quality contract.
    Check(CheckOptions),
    /// Manage Ayni's agent instructions.
    Agents {
        #[command(subcommand)]
        command: AgentsCommands,
    },
    /// Inspect and compare explicit local result files.
    Results {
        #[command(subcommand)]
        command: ResultsCommands,
    },
    #[command(hide = true)]
    GenerateDocs,
}

impl Commands {
    fn into_operation(self) -> Operation {
        match self {
            Self::ValidateSeeds => Operation::ValidateSeeds,
            Self::PreparedExec { command } => Operation::PreparedExec(command),
            Self::Init(options) => Operation::Init(options.into_operation()),
            Self::Env { command } => command.into_operation(),
            Self::Tools {
                command: ToolsCommands::Reconcile(options),
            } => Operation::ToolsReconcile(crate::application::ToolsReconcileOperation {
                config: options.config,
                repo_root: options.repo_root,
                output: options.output.into(),
                check: options.check,
            }),
            Self::Contract { command } => command.into_operation(),
            Self::Verify { command } => command.into_operation(),
            Self::Impact { command } => command.into_operation(),
            Self::Check(options) => Operation::Check(options.into_operation()),
            Self::Agents {
                command: AgentsCommands::Sync(options),
            } => Operation::AgentsSync(options.into()),
            Self::Results { command } => command.into_operation(),
            Self::GenerateDocs => Operation::GenerateDocs,
        }
    }
}

#[derive(Subcommand, Debug)]
enum EnvCommands {
    /// Explain the resolved environment plan without modifying state.
    Show(EnvShowOptions),
    /// Diagnose missing, conflicting, unsupported, or stale environment state.
    Doctor(RepositoryOptions),
    /// Resolve exact environment requirements into the committed lock.
    Lock(EnvLockOptions),
    /// Build the repository code-environment image from a current lock.
    Build(EnvBuildOptions),
    /// Certify a staged filesystem without running repository code or a container engine.
    Certify(EnvCertifyOptions),
    /// Report Ayni-managed OCI images and repository-local environment state.
    Storage(EnvStorageOptions),
    /// Preview or remove stale repository state and explicitly selected images.
    Prune(EnvPruneOptions),
}

impl EnvCommands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Show(options) => Operation::EnvShow(options.into_operation()),
            Self::Doctor(options) => Operation::EnvDoctor(options.into()),
            Self::Lock(options) => Operation::EnvLock(options.into_operation()),
            Self::Build(options) => Operation::EnvBuild(crate::application::EnvBuildOperation {
                repo_root: options.repo_root,
                executor_image: options.executor_image,
                tag: options.tag,
                cache_from: options.cache_from,
                cache_to: options.cache_to,
            }),
            Self::Certify(options) => Operation::EnvCertify(options.into_operation()),
            Self::Storage(options) => Operation::EnvStorage(options.into_operation()),
            Self::Prune(options) => Operation::EnvPrune(options.into_operation()),
        }
    }
}

#[derive(Subcommand, Debug)]
enum ContractCommands {
    /// Render and validate the effective quality contract.
    Show(ContractOptions),
}

impl ContractCommands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Show(options) => Operation::ContractShow(options.into_operation()),
        }
    }
}

#[derive(Subcommand, Debug)]
enum VerifyCommands {
    /// List exact verification commands from a saved current-schema artifact.
    List(VerifyListOptions),
    /// Run only the test signal.
    Test {
        #[command(flatten)]
        options: VerifyFilePackageOptions,
        #[arg(long)]
        name: Option<String>,
    },
    /// Run only the coverage signal.
    Coverage(VerifyCommonOptions),
    /// Run only the size signal.
    Size(VerifyFileOptions),
    /// Run only the complexity signal.
    Complexity(VerifyFilePackageOptions),
    /// Run only the dependency signal.
    Deps(VerifyFilePackageOptions),
    /// Run only the mutation signal.
    Mutation(VerifyCommonOptions),
}

impl VerifyCommands {
    fn into_operation(self) -> Operation {
        let command = match self {
            Self::List(options) => {
                return Operation::VerifyList(VerifyListOperation {
                    artifact: options.artifact,
                });
            }
            command => command,
        };
        let (signal, options, file, package, name) = match command {
            Self::List(_) => unreachable!("verify list handled before signal dispatch"),
            Self::Test { options, name } => (
                SignalKind::Test,
                options.common,
                options.file,
                options.package,
                name,
            ),
            Self::Coverage(options) => (SignalKind::Coverage, options, None, None, None),
            Self::Size(options) => (SignalKind::Size, options.common, options.file, None, None),
            Self::Complexity(options) => (
                SignalKind::Complexity,
                options.common,
                options.file,
                options.package,
                None,
            ),
            Self::Deps(options) => (
                SignalKind::Deps,
                options.common,
                options.file,
                options.package,
                None,
            ),
            Self::Mutation(options) => (SignalKind::Mutation, options, None, None, None),
        };
        Operation::Verify(VerifyOperation {
            signal,
            config: options.config,
            language: options.language.map(LanguageArg::into_language),
            root: options.root,
            file,
            package,
            name,
            output: options.output.into(),
            debug: options.debug,
        })
    }
}

#[derive(Subcommand, Debug)]
enum ImpactCommands {
    /// Explain the quality work affected by a change without running it.
    Show(ImpactShowOptions),
    /// Execute the quality work affected by a change.
    Run(ImpactRunOptions),
}

impl ImpactCommands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Show(options) => Operation::ImpactShow(options.into_operation()),
            Self::Run(options) => Operation::ImpactRun(options.into_operation()),
        }
    }
}

#[derive(Subcommand, Debug)]
enum AgentsCommands {
    /// Create or refresh only Ayni's managed AGENTS.md block.
    Sync(RepositoryOptions),
}

#[derive(Subcommand, Debug)]
enum ResultsCommands {
    /// Compare two explicit compatible result files.
    Compare(ResultsCompareOptions),
}

impl ResultsCommands {
    fn into_operation(self) -> Operation {
        match self {
            Self::Compare(options) => Operation::ResultsCompare(ResultsCompareOperation {
                baseline: options.baseline,
                candidate: options.candidate,
                output: options.output.into(),
            }),
        }
    }
}

#[derive(Args, Debug)]
struct InitOptions {
    /// Repository root to inspect and where `.ayni.toml` will be written.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Print the proposed policy without writing it (the default behavior).
    #[arg(long, conflicts_with = "write")]
    dry_run: bool,
    /// Create `.ayni.toml`; refuses to overwrite an existing policy.
    #[arg(long, conflicts_with = "dry_run")]
    write: bool,
}

impl InitOptions {
    fn into_operation(self) -> InitOperation {
        InitOperation {
            repo_root: self.repo_root,
            write: self.write && !self.dry_run,
        }
    }
}

#[derive(Args, Debug)]
struct RepositoryOptions {
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
}

impl From<RepositoryOptions> for RepositoryOperation {
    fn from(options: RepositoryOptions) -> Self {
        Self {
            repo_root: options.repo_root,
        }
    }
}

#[derive(Args, Debug)]
struct EnvShowOptions {
    /// Policy configuration file, resolved under the repository root.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Repository root that contains the policy and configured targets.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Render a human-readable plan or one JSON document.
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
}

impl EnvShowOptions {
    fn into_operation(self) -> EnvShowOperation {
        EnvShowOperation {
            config: self.config,
            repo_root: self.repo_root,
            output: self.output.into(),
        }
    }
}

#[derive(Args, Debug)]
struct EnvLockOptions {
    /// Policy configuration file, resolved under the repository root.
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Repository root where `.ayni.lock` will be written.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Exact provisioning base as `<reference>@sha256:<digest>`; defaults to pinned upstream Debian.
    #[arg(long)]
    base: Option<String>,
}

#[derive(Args, Debug)]
struct EnvCertifyOptions {
    /// Frozen repository whose committed policy and lock are validated without adapter discovery.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Read-only candidate filesystem, including `/` when running inside that candidate.
    #[arg(long)]
    filesystem_root: PathBuf,
    /// Independently selected Linux platform; certification must run on this native platform.
    #[arg(long, value_parser = ["linux/amd64", "linux/arm64"])]
    platform: String,
    /// Trusted expected Ayni executable identity as `sha256:<64 lowercase hex characters>`.
    #[arg(long)]
    expected_ayni_sha256: String,
    /// Independently reviewed protected-content root from frozen unsigned-build evidence.
    #[arg(long)]
    expected_content_root: String,
    /// Public certificate key identifier pinned by the committed trust policy.
    #[arg(long)]
    key_id: String,
    /// Explicit regular file containing a 64-character lowercase hexadecimal Ed25519 seed.
    /// Keep it outside frozen source and the staged filesystem. With `/`, use a separate mount outside protected paths.
    #[arg(long)]
    signing_key_file: PathBuf,
    /// Destination metadata layer archive. A structured certification report is written to stdout.
    #[arg(long)]
    output: PathBuf,
}

impl EnvCertifyOptions {
    fn into_operation(self) -> EnvCertifyOperation {
        EnvCertifyOperation {
            repo_root: self.repo_root,
            filesystem_root: self.filesystem_root,
            platform: self.platform,
            expected_ayni_sha256: self.expected_ayni_sha256,
            expected_content_root: self.expected_content_root,
            key_id: self.key_id,
            signing_key_file: self.signing_key_file,
            output: self.output,
        }
    }
}

impl EnvLockOptions {
    fn into_operation(self) -> EnvLockOperation {
        EnvLockOperation {
            config: self.config,
            repo_root: self.repo_root,
            base: self.base,
        }
    }
}

#[derive(Args, Debug)]
struct EnvStorageOptions {
    /// Repository root whose current lock and materialized state are inspected.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Render a human-readable report or one JSON document.
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
}

impl EnvStorageOptions {
    fn into_operation(self) -> EnvStorageOperation {
        EnvStorageOperation {
            repo_root: self.repo_root,
            output: self.output.into(),
        }
    }
}

#[derive(Args, Debug)]
struct EnvPruneOptions {
    /// Repository root whose current environment is retained unless --current is set.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Render a human-readable report or one JSON document.
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
    /// Remove selected candidates; without this flag the command is read-only.
    #[arg(long)]
    apply: bool,
    /// Include non-current Ayni-owned images across the selected engine.
    ///
    /// Images can be shared by multiple repositories. This flag explicitly
    /// acknowledges the engine-wide scope; without it, only repository-local
    /// stale state is selected for removal.
    #[arg(long)]
    images: bool,
    /// Include the current repository-local environment state.
    ///
    /// The next managed command recreates it from the locked image. Current
    /// images remain retained because they can be shared across repositories.
    #[arg(long)]
    current: bool,
}

impl EnvPruneOptions {
    fn into_operation(self) -> EnvPruneOperation {
        EnvPruneOperation {
            repo_root: self.repo_root,
            output: self.output.into(),
            apply: self.apply,
            images: self.images,
            current: self.current,
        }
    }
}

#[derive(Args, Debug)]
struct ContractOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
}

impl ContractOptions {
    fn into_operation(self) -> ContractOperation {
        ContractOperation {
            config: self.config,
            output: self.output.into(),
        }
    }
}

#[derive(Args, Debug)]
struct CheckOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    #[arg(long, value_enum, default_value_t)]
    output: OutputArg,
    /// Print raw command diagnostics.
    #[arg(long)]
    debug: bool,
}

impl CheckOptions {
    fn into_operation(self) -> CheckOperation {
        CheckOperation {
            config: self.config,
            output: self.output.into(),
            debug: self.debug,
        }
    }
}

#[derive(Args, Debug)]
struct VerifyListOptions {
    /// Saved schema-v4 result artifact to inspect.
    #[arg(long, default_value = DEFAULT_RESULTS_ARTIFACT)]
    artifact: PathBuf,
}

#[derive(Args, Debug)]
struct VerifyCommonOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    #[arg(long, value_enum)]
    language: Option<LanguageArg>,
    /// Select exactly one normalized root configured for the selected language.
    #[arg(long)]
    root: Option<String>,
    #[arg(long, value_enum, default_value_t)]
    output: OutputArg,
    /// Print raw command diagnostics.
    #[arg(long)]
    debug: bool,
}

#[derive(Args, Debug)]
struct VerifyFileOptions {
    #[command(flatten)]
    common: VerifyCommonOptions,
    #[arg(long)]
    file: Option<String>,
}

#[derive(Args, Debug)]
struct VerifyFilePackageOptions {
    #[command(flatten)]
    common: VerifyCommonOptions,
    #[arg(long)]
    file: Option<String>,
    #[arg(long)]
    package: Option<String>,
}

#[derive(Args, Debug)]
struct ImpactShowOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    /// Explicit base revision used to calculate the change.
    #[arg(long)]
    base: String,
    #[arg(long, value_enum, default_value_t)]
    output: OutputArg,
}

impl ImpactShowOptions {
    fn into_operation(self) -> ImpactOperation {
        ImpactOperation {
            config: self.config,
            base: self.base,
            output: self.output.into(),
            debug: false,
        }
    }
}

#[derive(Args, Debug)]
struct ImpactRunOptions {
    #[command(flatten)]
    common: ImpactShowOptions,
    /// Print raw command diagnostics.
    #[arg(long)]
    debug: bool,
}

impl ImpactRunOptions {
    fn into_operation(self) -> ImpactOperation {
        let mut operation = self.common.into_operation();
        operation.debug = self.debug;
        operation
    }
}

#[derive(Args, Debug)]
struct ResultsCompareOptions {
    /// Earlier result file.
    #[arg(long)]
    baseline: PathBuf,
    /// Later result file.
    #[arg(long)]
    candidate: PathBuf,
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum DataOutputArg {
    /// Human-readable terminal output.
    #[default]
    Human,
    /// One deterministic JSON document on stdout.
    Json,
}

impl From<DataOutputArg> for OutputFormat {
    fn from(output: DataOutputArg) -> Self {
        match output {
            DataOutputArg::Human => Self::Human,
            DataOutputArg::Json => Self::Json,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum OutputArg {
    /// Human-readable terminal output.
    #[default]
    Human,
    /// One deterministic JSON document on stdout.
    Json,
    /// Deterministic Markdown output.
    Markdown,
}

impl From<OutputArg> for OutputFormat {
    fn from(output: OutputArg) -> Self {
        match output {
            OutputArg::Human => Self::Human,
            OutputArg::Json => Self::Json,
            OutputArg::Markdown => Self::Markdown,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum LanguageArg {
    Rust,
    Go,
    Node,
    Python,
    Kotlin,
}

impl LanguageArg {
    fn into_language(self) -> Language {
        match self {
            Self::Rust => Language::Rust,
            Self::Go => Language::Go,
            Self::Node => Language::Node,
            Self::Python => Language::Python,
            Self::Kotlin => Language::Kotlin,
        }
    }
}

#[derive(Args, Debug)]
struct EnvBuildOptions {
    /// Repository containing the committed environment lock.
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    /// Use this immutable builder image to supply Ayni without changing the environment lock.
    #[arg(long, value_name = "REFERENCE@sha256:DIGEST")]
    executor_image: Option<String>,
    /// Local tag applied to the built image. This does not publish it.
    #[arg(long)]
    tag: Option<String>,
    /// Import an external Buildx cache; repeat for multiple sources. Does not change the lock.
    #[arg(long, value_name = "CACHE")]
    cache_from: Vec<String>,
    /// Export an external Buildx cache; repeat for multiple destinations. Does not change the lock.
    #[arg(long, value_name = "CACHE")]
    cache_to: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use crate::application::{Operation, OutputFormat};
    use ayni_core::{Language, SignalKind};
    use clap::{CommandFactory, Parser};
    use std::path::PathBuf;

    #[test]
    fn complete_command_tree_is_exposed() {
        let command = Cli::command();
        let names = command
            .get_subcommands()
            .filter(|subcommand| !subcommand.is_hide_set())
            .map(|subcommand| subcommand.get_name())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "init", "env", "tools", "contract", "verify", "impact", "check", "agents",
                "results"
            ]
        );
    }

    #[test]
    fn every_public_command_maps_to_one_typed_operation() {
        let cases = [
            (vec!["ayni", "init", "--dry-run"], "Init"),
            (vec!["ayni", "env", "show"], "EnvShow"),
            (vec!["ayni", "env", "doctor"], "EnvDoctor"),
            (vec!["ayni", "env", "lock"], "EnvLock"),
            (vec!["ayni", "env", "build"], "EnvBuild"),
            (
                vec![
                    "ayni",
                    "env",
                    "certify",
                    "--filesystem-root",
                    "/candidate",
                    "--platform",
                    "linux/arm64",
                    "--expected-ayni-sha256",
                    "sha256:abc",
                    "--expected-content-root",
                    "sha256:def",
                    "--key-id",
                    "builder",
                    "--signing-key-file",
                    "/signer/key",
                    "--output",
                    "/output/layer.tar",
                ],
                "EnvCertify",
            ),
            (vec!["ayni", "env", "storage"], "EnvStorage"),
            (vec!["ayni", "env", "prune"], "EnvPrune"),
            (vec!["ayni", "contract", "show"], "ContractShow"),
            (vec!["ayni", "tools", "reconcile"], "ToolsReconcile"),
            (vec!["ayni", "verify", "list"], "VerifyList"),
            (vec!["ayni", "verify", "test"], "Verify"),
            (
                vec!["ayni", "impact", "show", "--base", "main"],
                "ImpactShow",
            ),
            (vec!["ayni", "impact", "run", "--base", "main"], "ImpactRun"),
            (vec!["ayni", "check"], "Check"),
            (vec!["ayni", "agents", "sync"], "AgentsSync"),
            (
                vec![
                    "ayni",
                    "results",
                    "compare",
                    "--baseline",
                    "before.json",
                    "--candidate",
                    "after.json",
                ],
                "ResultsCompare",
            ),
        ];

        for (arguments, expected) in cases {
            let operation = Cli::try_parse_from(arguments)
                .expect("arguments parse")
                .into_operation();
            assert!(
                format!("{operation:?}").starts_with(expected),
                "{operation:?}"
            );
        }
    }

    #[test]
    fn filesystem_certification_requires_explicit_identity_key_file_and_output() {
        let arguments = [
            "ayni",
            "env",
            "certify",
            "--repo-root",
            "/source",
            "--filesystem-root",
            "/candidate",
            "--platform",
            "linux/arm64",
            "--expected-ayni-sha256",
            "sha256:abc",
            "--expected-content-root",
            "sha256:def",
            "--key-id",
            "builder",
            "--signing-key-file",
            "/signer/key",
            "--output",
            "/output/layer.tar",
        ];
        let operation = Cli::try_parse_from(arguments)
            .expect("certification arguments")
            .into_operation();
        let Operation::EnvCertify(operation) = operation else {
            panic!("certification operation")
        };
        assert_eq!(operation.repo_root, PathBuf::from("/source"));
        assert_eq!(operation.filesystem_root, PathBuf::from("/candidate"));
        assert_eq!(operation.platform, "linux/arm64");
        assert_eq!(operation.expected_ayni_sha256, "sha256:abc");
        assert_eq!(operation.expected_content_root, "sha256:def");
        assert_eq!(operation.key_id, "builder");
        assert_eq!(operation.signing_key_file, PathBuf::from("/signer/key"));
        assert_eq!(operation.output, PathBuf::from("/output/layer.tar"));
        for flag in [
            "--filesystem-root",
            "--platform",
            "--expected-ayni-sha256",
            "--expected-content-root",
            "--key-id",
            "--signing-key-file",
            "--output",
        ] {
            let index = arguments
                .iter()
                .position(|value| *value == flag)
                .expect("flag");
            let incomplete = arguments
                .iter()
                .enumerate()
                .filter_map(|(i, value)| (i != index && i != index + 1).then_some(*value));
            assert!(
                Cli::try_parse_from(incomplete).is_err(),
                "{flag} must be explicit"
            );
        }
    }

    #[test]
    fn environment_storage_and_prune_map_output_and_apply_explicitly() {
        let storage = Cli::try_parse_from([
            "ayni",
            "env",
            "storage",
            "--repo-root",
            "fixture",
            "--output",
            "json",
        ])
        .expect("storage arguments")
        .into_operation();
        let Operation::EnvStorage(storage) = storage else {
            panic!("storage operation");
        };
        assert_eq!(storage.repo_root, PathBuf::from("fixture"));
        assert_eq!(storage.output, OutputFormat::Json);

        let prune = Cli::try_parse_from([
            "ayni",
            "env",
            "prune",
            "--repo-root",
            "fixture",
            "--apply",
            "--images",
        ])
        .expect("prune arguments")
        .into_operation();
        let Operation::EnvPrune(prune) = prune else {
            panic!("prune operation");
        };
        assert_eq!(prune.repo_root, PathBuf::from("fixture"));
        assert_eq!(prune.output, OutputFormat::Human);
        assert!(prune.apply);
        assert!(prune.images);
        assert!(!prune.current);

        let Operation::EnvPrune(current) =
            Cli::try_parse_from(["ayni", "env", "prune", "--current"])
                .expect("current prune arguments")
                .into_operation()
        else {
            panic!("current prune operation");
        };
        assert!(current.current);

        let Operation::EnvPrune(defaults) = Cli::try_parse_from(["ayni", "env", "prune"])
            .expect("default prune arguments")
            .into_operation()
        else {
            panic!("prune operation");
        };
        assert!(!defaults.apply);
        assert!(!defaults.images);
        assert!(!defaults.current);
    }

    #[test]
    fn verify_list_defaults_to_the_last_repository_artifact() {
        let operation = Cli::try_parse_from(["ayni", "verify", "list"])
            .expect("arguments parse")
            .into_operation();
        let Operation::VerifyList(operation) = operation else {
            panic!("verify list operation");
        };
        assert_eq!(
            operation.artifact,
            PathBuf::from("./.ayni/last/signals.json")
        );
    }

    #[test]
    fn verify_maps_signal_selectors_and_global_execution_options() {
        let operation = Cli::try_parse_from([
            "ayni",
            "verify",
            "test",
            "--config",
            "policy.toml",
            "--language",
            "node",
            "--root",
            "frontend",
            "--package",
            "@example/web",
            "--file",
            "frontend/cart.test.ts",
            "--name",
            "formats money",
            "--output",
            "json",
        ])
        .expect("arguments parse")
        .into_operation();
        let Operation::Verify(operation) = operation else {
            panic!("verify operation");
        };
        assert_eq!(operation.signal, SignalKind::Test);
        assert_eq!(operation.config, PathBuf::from("policy.toml"));
        assert_eq!(operation.language, Some(Language::Node));
        assert_eq!(operation.root.as_deref(), Some("frontend"));
        assert_eq!(operation.package.as_deref(), Some("@example/web"));
        assert_eq!(operation.file.as_deref(), Some("frontend/cart.test.ts"));
        assert_eq!(operation.name.as_deref(), Some("formats money"));
        assert_eq!(operation.output, OutputFormat::Json);
    }

    #[test]
    fn signal_invalid_selectors_are_rejected_before_execution() {
        for (signal, selector) in [
            ("coverage", "--file"),
            ("size", "--package"),
            ("mutation", "--name"),
        ] {
            let error = Cli::try_parse_from(["ayni", "verify", signal, selector, "target"])
                .expect_err("invalid selector must fail");
            assert!(error.to_string().contains("unexpected argument"));
        }
    }

    #[test]
    fn quality_commands_have_one_execution_mode() {
        assert!(Cli::try_parse_from(["ayni", "check", "--host"]).is_err());
        assert!(Cli::try_parse_from(["ayni", "verify", "test", "--host"]).is_err());
        assert!(
            Cli::try_parse_from(["ayni", "impact", "run", "--base", "main", "--host"]).is_err()
        );
        assert!(Cli::try_parse_from(["ayni", "env", "shell"]).is_err());
        assert!(Cli::try_parse_from(["ayni", "env", "run", "--", "cargo", "test"]).is_err());
    }

    #[test]
    fn output_support_is_explicit_per_command() {
        assert!(Cli::try_parse_from(["ayni", "contract", "show", "--output", "markdown"]).is_err());
        assert!(
            Cli::try_parse_from([
                "ayni",
                "results",
                "compare",
                "--baseline",
                "before.json",
                "--candidate",
                "after.json",
                "--output",
                "markdown",
            ])
            .is_err()
        );
        assert!(Cli::try_parse_from(["ayni", "check", "--output", "markdown"]).is_ok());
    }

    #[test]
    fn impact_show_rejects_execution_only_options() {
        for option in ["--host", "--debug"] {
            assert!(
                Cli::try_parse_from(["ayni", "impact", "show", "--base", "main", option]).is_err(),
                "{option} must remain exclusive to impact run"
            );
        }
        assert!(
            Cli::try_parse_from(["ayni", "impact", "run", "--base", "main", "--debug"]).is_ok()
        );
        assert!(
            Cli::try_parse_from(["ayni", "impact", "run", "--base", "main", "--host"]).is_err()
        );
    }

    #[test]
    fn superseded_commands_do_not_parse() {
        for command in ["analyze", "install", "artifact"] {
            let error = Cli::try_parse_from(["ayni", command]).expect_err("old command must fail");
            assert!(error.to_string().contains("unrecognized subcommand"));
        }
        assert!(Cli::try_parse_from(["ayni", "contract", "display"]).is_err());
    }
}

#[derive(Subcommand, Debug)]
enum ToolsCommands {
    /// Inspect native tooling without changing files or executing package managers.
    Reconcile(ToolsReconcileOptions),
}
#[derive(Args, Debug)]
struct ToolsReconcileOptions {
    #[arg(long, default_value = DEFAULT_CONFIG)]
    config: PathBuf,
    #[arg(long, default_value = ".")]
    repo_root: PathBuf,
    #[arg(long, value_enum, default_value_t)]
    output: DataOutputArg,
    /// Fail when reconciliation is required or inspection is blocked.
    #[arg(long)]
    check: bool,
}
