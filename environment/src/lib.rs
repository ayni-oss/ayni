//! Lock-driven OCI image planning and launching for Ayni managed environments.
//!
//! This layer consumes validated core lock contracts. Language adapters remain
//! responsible for ecosystem semantics; the CLI owns user intent and rendering.

use std::path::Path;
use std::process::Output;
use std::time::Duration;

mod certificate;
pub use certificate::validate_signing_trust;
pub use certificate::{
    MAX_CERTIFICATE_BYTES, MAX_CERTIFICATE_KEY_ID_BYTES, MAX_PROTECTED_MANIFEST_BYTES,
    MAX_PROTECTED_MANIFEST_ENTRIES, PROTECTED_ANCESTOR_DIRS, PROTECTED_FILE_ROOTS,
    PROTECTED_TREE_ROOTS, ProtectedManifestEntry, parse_protected_manifest,
};
mod executor;
pub use executor::execution_build_record;
mod image;
mod lock;
pub mod prebuilt;
mod preparation;
mod preparation_groups;
mod runtime;
mod storage;

pub use image::{ImagePlan, image_plan, signal_tool_coordinate};
pub use lock::{
    BASE_MISE_VERSION, BASE_VARIANT, LOCK_FILE, plan_matches_lock, read_lock,
    resolve_provisioning_base,
};
pub use runtime::{
    BuildCache, Engine, build, build_prepared, build_prepared_with_cache,
    build_prepared_with_executor, detect_engine, doctor, doctor_prepared, locked_tool_versions,
    target_environment,
};
pub use storage::{
    StorageImage, StorageImageOwnership, StorageImagePruneScope, StoragePruneFailure,
    StoragePruneResult, StorageReport, StorageStateGeneration, prune_storage,
    prune_storage_prepared, storage_report, storage_report_prepared,
};

pub(crate) fn run_oci_command(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<Output, String> {
    ayni_adapters_common::exec::run_command(workdir, program, args, timeout)
}

pub(crate) fn run_oci_command_streaming_truncated(
    workdir: &Path,
    program: &str,
    args: &[String],
    timeout: Duration,
    on_line: impl FnMut(&str),
) -> Result<ayni_adapters_common::exec::TruncatedOutput, String> {
    ayni_adapters_common::exec::run_command_streaming_truncated(
        workdir, program, args, timeout, on_line,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendErrorKind {
    Input,
    Environment,
    Execution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub kind: BackendErrorKind,
    pub message: String,
}

impl BackendError {
    pub fn input(message: impl Into<String>) -> Self {
        Self {
            kind: BackendErrorKind::Input,
            message: message.into(),
        }
    }

    pub fn environment(message: impl Into<String>) -> Self {
        Self {
            kind: BackendErrorKind::Environment,
            message: message.into(),
        }
    }

    pub fn execution(message: impl Into<String>) -> Self {
        Self {
            kind: BackendErrorKind::Execution,
            message: message.into(),
        }
    }
}

pub(crate) fn concise_output(bytes: &[u8]) -> String {
    let output = String::from_utf8_lossy(bytes);
    let mut lines = output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(8)
        .collect::<Vec<_>>();
    lines.reverse();
    if lines.is_empty() {
        String::from("command failed without diagnostics")
    } else {
        let value = lines.join("\n");
        value.chars().take(4000).collect()
    }
}
