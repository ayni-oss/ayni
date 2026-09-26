# Provisioning and repository automation

Ayni uses one composed repository environment whose durable provisioning
substrate is independent of the executor. The `ayni-provisioning` image contains
Debian prerequisites, Mise, and the execution user; it does not contain an Ayni
executable or language-specific runtime.

## Provisioning contract

`.github/docker/provisioning.versions` records the immutable Debian and Mise
inputs used by the substrate. `environment/provisioning.json` is the
authoritative substrate reference embedded in Ayni, and `.ayni.lock` binds the
repository contract to an exact published digest.

A substrate digest must be built for every supported architecture, scanned, and
publicly pullable before it is adopted. Updating the committed reference is a
separate reviewed change. The repository no longer publishes provisioning
images from a GitHub Actions workflow.

Environment lock schema `0.7.0` separates the provisioning substrate from the
executor identity. Regenerate a changed lock twice with the checkout CLI and
require byte-for-byte equality before committing it.

## Pull-request validation

`.github/workflows/pr-validation.yml` is the single pull-request workflow. It:

- validates Conventional Commit title syntax and maintains the corresponding
  `kind/*` label;
- verifies the project files required by the repository's CNCF-readiness policy;
- installs the pinned public Ayni CLI, builds and verifies the committed managed
  environment, and runs `ayni check`; and
- creates or updates one marked Ayni results comment on every pull-request run.

The Ayni execution job has read-only repository permission. Label and comment
writes happen in separate jobs so pull-request code does not run with a write
credential. The comment job consumes only the Markdown report artifact and
updates the existing marked comment instead of appending a new comment on each
synchronization.

Run affected fixtures and specialized delivery checks locally when changing
adapter, environment, installer, or publication behavior. The pull-request
workflow intentionally represents only the repository's declared Ayni contract.

## CLI release

`.github/workflows/release.yml` uses Release Please on `main` and supports manual
recovery for an existing release tag. When a release is created or selected, it
calls `.github/workflows/release-publication.yml` to build the supported macOS
and Linux CLI archives, attest them, generate checksums, and upload the assets.

Release publication uses immutable tagged source and remains recoverable for an
existing public release. The archive naming contract is
`ayni-<release-tag>-<target>.tar.gz`.
