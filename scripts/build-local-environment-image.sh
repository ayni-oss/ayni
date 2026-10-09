#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
versions_file="$repo_root/.github/docker/ayni-builder.versions"
# shellcheck disable=SC1090
source "$versions_file"

docker_arch="$(docker info --format '{{.Architecture}}')"
case "$docker_arch" in
  amd64|x86_64) platform_arch="amd64" ;;
  arm64|aarch64) platform_arch="arm64" ;;
  *)
    echo "unsupported Docker architecture: $docker_arch" >&2
    exit 2
    ;;
esac

source_revision="$(git -C "$repo_root" rev-parse HEAD)"
ayni_version="$(cargo metadata --manifest-path "$repo_root/Cargo.toml" --locked --no-deps --format-version 1 | jq -r '.packages[] | select(.name == "ayni-cli") | .version')"

# Compile inside Linux so the binary copied into the Debian image is ELF,
# rather than the host's macOS Mach-O executable.
docker run --rm \
  --platform "linux/$platform_arch" \
  --user "$(id -u):$(id -g)" \
  --env CARGO_HOME=/tmp/cargo \
  --volume "$repo_root:/workspace" \
  --workdir /workspace \
  "$RUST_BUILDER_IMAGE" \
  cargo build --locked -p ayni-cli --release

context="$(mktemp -d "${TMPDIR:-/tmp}/ayni-builder-context.XXXXXX")"
trap 'rm -rf "$context"' EXIT
cp "$repo_root/target/release/ayni" "$context/ayni"
cp "$repo_root/LICENSE" "$context/"
cp "$repo_root/.github/docker/ayni-builder.Dockerfile" "$context/"

docker build \
  --provenance=false \
  --platform "linux/$platform_arch" \
  --build-arg "DEBIAN_IMAGE=$DEBIAN_IMAGE" \
  --build-arg "AYNI_VERSION=$ayni_version" \
  --build-arg "SOURCE_REVISION=$source_revision" \
  --file "$context/ayni-builder.Dockerfile" \
  --tag ayni-builder:local \
  "$context"

base_id="$(docker image inspect ayni-builder:local --format '{{.Id}}')"
base_reference="$(docker image inspect ayni-builder:local --format '{{if .RepoDigests}}{{index .RepoDigests 0}}{{end}}')"
case "$base_reference" in
  */*@sha256:*) ;;
  *) base_reference='' ;;
esac
printf '\nBuilt ayni-builder:local (%s)\n' "$base_id"
if [[ -n "$base_reference" ]]; then
  printf 'Next: cargo run -p ayni-cli -- env build --executor-image "%s"\n' "$base_reference"
else
  printf '%s\n' \
    'This engine did not expose a repository manifest digest for the local tag.' \
    'Push the image to a local registry, then pass its exact RepoDigest to env build --executor-image.'
fi
