# Provisioning and repository automation

Ayni publishes one container image per CLI release:
`ghcr.io/ayni-oss/ayni-builder:<version>`, for Linux amd64 and arm64. The builder
contains Ayni, Docker CLI, Buildx, Git and CA certificates. It connects to an
external engine and does not contain a Docker daemon or project toolchains.
Standalone CLI archives remain available for macOS and Linux.

## Provisioning contract

`environment/provisioning.json` records the immutable upstream Debian base used
by new locks. `.github/docker/provisioning.versions` pins Debian and Mise inputs.
`ayni env build` generates instructions that install OS prerequisites, verify the
Mise download checksum, create the execution user, and install locked tools and
prepare dependencies inside the produced image. Mise is not required in the
factory container when building an existing lock.

The version-matched builder supplies only its Ayni executable to the produced
image. Docker and Buildx are not copied into repository environments. No separate
Ayni CLI or provisioning image is published. Existing locks referencing a
labelled provisioning image remain supported; their immutable inputs do not
change until the lock is explicitly regenerated.

The builder recipe is `.github/docker/ayni-builder.Dockerfile`. Pull-request
candidates, local builds, and release publication all use this recipe. Run
`scripts/build-local-environment-image.sh` to build `ayni-builder:local`.

Regenerate a changed lock twice with the checkout CLI and require byte-for-byte
equality before committing it.

## Pull-request validation

`.github/workflows/pr-validation.yml` is the single pull-request workflow. It:

- validates Conventional Commit title syntax and maintains the corresponding
  `kind/*` label;
- verifies the project files required by the repository's CNCF-readiness policy;
- installs the pinned public Ayni CLI, builds and verifies the committed managed
  environment, and runs `ayni check`; and
- publishes the Markdown report and updates one marked Ayni results comment.

The Ayni execution job has read-only repository permission. Its comment job
does not check out or execute pull-request code; it consumes only the Markdown
report artifact and updates the existing marked comment instead of appending a
new comment on each synchronization.

Run affected fixtures and specialized delivery checks locally when changing
adapter, environment, installer, or publication behavior. The pull-request
workflow intentionally represents only the repository's declared Ayni contract.

## CLI release

`.github/workflows/release.yml` uses Release Please on `main` and supports manual
recovery for an existing release tag. When a release is created or selected, it
calls `.github/workflows/release-publication.yml` to build the supported macOS
and Linux CLI archives, attest them, generate checksums, upload the assets, and
publish `ghcr.io/ayni-oss/ayni-builder:<version>` for Linux amd64 and arm64.
The builder tag is validated anonymously and its digest is the immutable
identity Ayni records in an environment build.

Release publication uses immutable tagged source and remains recoverable for an
existing public release. The archive naming contract is
`ayni-<release-tag>-<target>.tar.gz`.

For recovery of a tag predating the builder recipe, binary compilation retains
that tag's original compiler inputs. Factory packaging uses the immutable
publication-workflow revision's builder recipe. The image records the tagged
CLI source in `org.opencontainers.image.revision` and the factory recipe source
in `dev.ayni.builder.recipe-revision`. Tags that contain the builder recipe use
the tagged recipe directly.
