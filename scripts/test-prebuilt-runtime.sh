#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

usage() {
  cat <<'EOF'
Usage: scripts/test-prebuilt-runtime.sh [--cli PATH --executor REFERENCE@sha256:DIGEST]

Without arguments, builds the host CLI and a matching Linux executor from the
current checkout. With arguments, reuses an already-built checkout CLI and
immutable executor, as CI does.
EOF
}

cli=''
executor=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    --cli)
      cli="${2:?--cli requires a path}"
      shift 2
      ;;
    --executor)
      executor="${2:?--executor requires an immutable image reference}"
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      echo "unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done
if [[ -n "$cli" || -n "$executor" ]]; then
  if [[ -z "$cli" || -z "$executor" ]]; then
    echo '--cli and --executor must be supplied together' >&2
    exit 2
  fi
  cli="$(cd "$(dirname "$cli")" && pwd)/$(basename "$cli")"
  case "$executor" in
    *@sha256:*) ;;
    *) echo '--executor must use an immutable @sha256: digest' >&2; exit 2 ;;
  esac
fi

command -v docker >/dev/null || { echo 'Docker is required' >&2; exit 2; }
docker info >/dev/null
command -v cargo >/dev/null || { echo 'Cargo is required for this contributor test' >&2; exit 2; }

scratch="$(mktemp -d "${TMPDIR:-/tmp}/ayni-prebuilt-runtime.XXXXXX")"
registry_name="ayni-prebuilt-runtime-registry-$$"
local_candidate=''
registry_candidate=''
final_image=''
cleanup() {
  if [[ -n "$final_image" ]]; then
    docker image rm --force "$final_image" >/dev/null 2>&1 || true
  fi
  if [[ -n "$registry_candidate" ]]; then
    docker image rm --force "$registry_candidate" >/dev/null 2>&1 || true
  fi
  if [[ -n "$local_candidate" ]]; then
    docker image rm --force "$local_candidate" >/dev/null 2>&1 || true
  fi
  docker rm --force "$registry_name" >/dev/null 2>&1 || true
  rm -rf "$scratch"
}
trap cleanup EXIT HUP INT TERM

platform_arch=''
case "$(docker info --format '{{.Architecture}}')" in
  amd64|x86_64) platform_arch=amd64 ;;
  arm64|aarch64) platform_arch=arm64 ;;
  *) echo 'Docker must provide amd64 or arm64 Linux containers' >&2; exit 2 ;;
esac
platform="linux/$platform_arch"

base="$(python3 - "$repo_root/.ayni.lock" <<'PY'
import json
import sys
lock = json.load(open(sys.argv[1]))
base = lock["provisioning_base"]
print(f'{base["reference"]}@{base["digest"]}')
PY
)"
base_mise_version="$(python3 - "$repo_root/.ayni.lock" <<'PY'
import json
import sys
lock = json.load(open(sys.argv[1]))
print(lock["provisioning_base"]["mise_version"])
PY
)"

if [[ -z "$cli" ]]; then
  source .github/docker/ayni-env.versions
  state="$repo_root/.ayni/prebuilt-runtime-e2e"
  mkdir -p "$state/host-target" "$state/linux-target" "$state/cargo"
  cargo build --locked --release -p ayni-cli --target-dir "$state/host-target"
  cli="$state/host-target/release/ayni"

  docker run --rm --platform "$platform" \
    --user "$(id -u):$(id -g)" \
    --env CARGO_HOME=/workspace/.ayni/prebuilt-runtime-e2e/cargo \
    --env CARGO_TARGET_DIR=/workspace/.ayni/prebuilt-runtime-e2e/linux-target \
    --volume "$repo_root:/workspace" --workdir /workspace \
    "$RUST_BUILDER_IMAGE" \
    cargo build --locked --release -p ayni-cli

  cp "$state/linux-target/release/ayni" "$scratch/ayni"
  cp LICENSE "$scratch/LICENSE"
  version="$($cli --version)"
  version="${version#ayni }"
  revision="$(git rev-parse HEAD)"
  local_candidate="ayni-prebuilt-runtime-candidate:$$"
  docker build --provenance=false --platform "$platform" \
    --build-arg "DEBIAN_IMAGE=$DEBIAN_IMAGE" \
    --build-arg "AYNI_VERSION=$version" \
    --build-arg "SOURCE_REVISION=$revision" \
    --file .github/docker/ayni-candidate.Dockerfile \
    --tag "$local_candidate" "$scratch"

  registry='localhost:5000'
  if ! curl --fail --silent "http://$registry/v2/" >/dev/null; then
    docker run --detach --rm --name "$registry_name" \
      --publish 127.0.0.1:5000:5000 \
      'registry:2.8.3@sha256:a3d8aaa63ed8681a604f1dea0aa03f100d5895b6a58ace528858a7b332415373' \
      >/dev/null
  fi
  for _ in {1..30}; do
    if curl --fail --silent "http://$registry/v2/" >/dev/null; then
      break
    fi
    sleep 1
  done
  curl --fail --silent "http://$registry/v2/" >/dev/null
  registry_candidate="$registry/ayni-candidate:checkout"
  docker tag "$local_candidate" "$registry_candidate"
  docker push "$registry_candidate" >/dev/null
  executor="$(docker image inspect "$registry_candidate" --format '{{range .RepoDigests}}{{println .}}{{end}}' \
    | grep -E "^${registry//./\.}/ayni-candidate@sha256:[0-9a-f]{64}$")"
  [[ "$(printf '%s\n' "$executor" | wc -l | tr -d ' ')" == 1 ]] \
    || { echo 'local registry did not return one immutable executor identity' >&2; exit 4; }
fi

# The fixture declares only exact versions, so locking needs mise solely to record
# its version. Keep this acceptance test independent of the contributor host and
# report the version of the pinned runtime base that the fixture will build from.
mkdir -p "$scratch/bin"
cat > "$scratch/bin/mise" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
if [[ "$#" -eq 4 \
  && "$1" == --no-config \
  && "$2" == --no-env \
  && "$3" == --no-hooks \
  && "$4" == version ]]; then
  printf '%s\n' "${AYNI_ACCEPTANCE_MISE_VERSION:?}"
  exit 0
fi
printf 'acceptance fixture mise shim received an unexpected invocation:' >&2
printf ' %q' "$@" >&2
printf '\n' >&2
exit 2
EOF
chmod 0755 "$scratch/bin/mise"
export AYNI_ACCEPTANCE_MISE_VERSION="$base_mise_version"
export PATH="$scratch/bin:$PATH"

fixture="$scratch/repository"
mkdir -p "$fixture/src" "$fixture/.ayni"
cat > "$fixture/.ayni.toml" <<'EOF'
[checks]
test = true
coverage = false
size = false
complexity = false
deps = false
mutation = false

[languages]
enabled = ["rust"]

[rust]
roots = ["."]

[environment.certificate.trusted_keys]
issue-35-test = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"

[environment.resources]
cpus = 2
memory_mib = 2048
memory_swap_mib = 2048
pids = 512
nofile = 1024
EOF
cat > "$fixture/Cargo.toml" <<'EOF'
[package]
name = "ayni-prebuilt-runtime-fixture"
version = "0.1.0"
edition = "2024"
EOF
cat > "$fixture/Cargo.lock" <<'EOF'
# This file is automatically @generated by Cargo.
# It is not intended for manual editing.
version = 4

[[package]]
name = "ayni-prebuilt-runtime-fixture"
version = "0.1.0"
EOF
cp rust-toolchain.toml "$fixture/rust-toolchain.toml"
cat > "$fixture/src/lib.rs" <<'EOF'
#[cfg(test)]
mod tests {
    #[test]
    fn certified_runtime_reaches_quality_execution() {
        assert_eq!(2 + 2, 4);
    }
}
EOF

"$cli" env lock --repo-root "$fixture" --base "$base"
export AYNI_ENV_CERTIFICATE_SIGNING_KEY=9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60
export AYNI_ENV_CERTIFICATE_KEY_ID=issue-35-test
"$cli" env build --repo-root "$fixture" --executor-image "$executor"
"$cli" env doctor --repo-root "$fixture"
final_image="$(python3 - "$fixture/.ayni/environment/build.json" <<'PY'
import json
import sys
print(json.load(open(sys.argv[1]))["image_tag"])
PY
)"

AYNI_PREBUILT_TEST_IMAGE="$final_image" \
AYNI_PREBUILT_TEST_SOURCE="$fixture" \
AYNI_PREBUILT_TEST_SIGNING_KEY="$AYNI_ENV_CERTIFICATE_SIGNING_KEY" \
  cargo test -p ayni-cli --test prebuilt_runtime_container_e2e -- --nocapture

printf '\nVerified env build -> certified image -> direct in-image check for %s.\n' "$platform"
