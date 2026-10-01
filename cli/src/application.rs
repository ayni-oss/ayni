use ayni_core::{Language, SignalKind};
use std::path::PathBuf;

// Internal carrier retained only by the legacy impact handoff code. It is not
// part of the command-line interface.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CapabilityAuthorization {
    pub allow_network: bool,
    pub allow_docker_socket: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputFormat {
    Human,
    Json,
    Markdown,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    Init(InitOperation),
    EnvShow(EnvShowOperation),
    EnvDoctor(RepositoryOperation),
    EnvLock(EnvLockOperation),
    EnvBuild(EnvBuildOperation),
    EnvStorage(EnvStorageOperation),
    EnvPrune(EnvPruneOperation),
    ContractShow(ContractOperation),
    ToolsReconcile(ToolsReconcileOperation),
    Verify(VerifyOperation),
    VerifyList(VerifyListOperation),
    ImpactShow(ImpactOperation),
    ImpactRun(ImpactOperation),
    Check(CheckOperation),
    AgentsSync(RepositoryOperation),
    ResultsCompare(ResultsCompareOperation),
    GenerateDocs,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct InitOperation {
    pub repo_root: PathBuf,
    pub write: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct RepositoryOperation {
    pub repo_root: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvShowOperation {
    pub config: PathBuf,
    pub repo_root: PathBuf,
    pub output: OutputFormat,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvLockOperation {
    pub config: PathBuf,
    pub repo_root: PathBuf,
    pub base: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvStorageOperation {
    pub repo_root: PathBuf,
    pub output: OutputFormat,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvPruneOperation {
    pub repo_root: PathBuf,
    pub output: OutputFormat,
    pub apply: bool,
    pub images: bool,
    pub current: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContractOperation {
    pub config: PathBuf,
    pub output: OutputFormat,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct CheckOperation {
    pub config: PathBuf,
    pub output: OutputFormat,
    pub debug: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifyListOperation {
    pub artifact: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifyOperation {
    pub signal: SignalKind,
    pub config: PathBuf,
    pub language: Option<Language>,
    pub root: Option<String>,
    pub file: Option<String>,
    pub package: Option<String>,
    pub name: Option<String>,
    pub output: OutputFormat,
    pub debug: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ImpactOperation {
    pub config: PathBuf,
    pub base: String,
    pub output: OutputFormat,
    pub debug: bool,
    pub execution_mode: ayni_core::ExecutionMode,
    pub authorization: CapabilityAuthorization,
    pub managed_handoff: Option<PathBuf>,
    pub managed_result: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ResultsCompareOperation {
    pub baseline: PathBuf,
    pub candidate: PathBuf,
    pub output: OutputFormat,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ToolsReconcileOperation {
    pub config: PathBuf,
    pub repo_root: PathBuf,
    pub output: OutputFormat,
    pub check: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct EnvBuildOperation {
    pub repo_root: PathBuf,
    pub executor_image: Option<String>,
    pub tag: Option<String>,
    pub cache_from: Vec<String>,
    pub cache_to: Vec<String>,
}
