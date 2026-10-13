#!/usr/bin/env bash
set -euo pipefail

cli="$(realpath "$AYNI_CI_CLI")"
platform="${AYNI_CI_PLATFORM:-linux/amd64}"
repository="$(printf '%s' "$AYNI_CI_REPOSITORY" | tr '[:upper:]' '[:lower:]')"
report="${AYNI_CI_REPORT:-${RUNNER_TEMP:-/tmp}/ayni.md}"
[[ "$repository" =~ ^[a-z0-9_.-]+/[a-z0-9_.-]+$ ]]
[[ "$platform" == linux/amd64 || "$platform" == linux/arm64 ]]
lock="$(jq -er '.fingerprint' .ayni.lock)"
[[ "$lock" =~ ^sha256:[0-9a-f]{64}$ ]]
[[ ! -L .ayni && ! -L .ayni/last ]]
rm -rf .ayni/last
mkdir -p "$(dirname "$report")"
: > "$report"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'report-path=%s\n' "$report" >> "$GITHUB_OUTPUT"
fi

scratch="$(mktemp -d)"
container=''
trap 'if [[ -n "$container" ]]; then docker rm "$container" >/dev/null 2>&1 || true; fi; rm -rf "$scratch"' EXIT
runtime=(docker run --rm --platform "$platform" --user "$(id -u):$(id -g)"
  --env HOME=/tmp/home --env XDG_CACHE_HOME=/tmp/cache --env MISE_CACHE_DIR=/tmp/mise
  --env CARGO_HOME=/tmp/cargo --env CARGO_TARGET_DIR=/tmp/target --env npm_config_cache=/tmp/npm
  --env GRADLE_USER_HOME=/tmp/gradle --env UV_CACHE_DIR=/tmp/uv
  --mount "type=bind,source=$PWD,target=/workspace" --workdir /workspace)

# Resolve mutable tags once, then inspect and run only their immutable image ID.
validate_image() {
  image="$(docker image inspect "$1" --format '{{.Id}}')" || return 1
  [[ "$image" =~ ^sha256:[0-9a-f]{64}$ ]] || return 1
  docker image inspect "$image" | jq -e --arg lock "$lock" --arg platform "$platform" '
    .[0] | (.Os + "/" + .Architecture) == $platform and
    .Config.Labels["dev.ayni.environment.platform"] == $platform and
    .Config.Labels["dev.ayni.environment.lock-fingerprint"] == $lock and
    .Config.Entrypoint == ["/usr/local/bin/ayni-entrypoint"]' >/dev/null || return 1
  container="$(docker create --platform "$platform" "$image")" || return 1
  docker cp "$container:/usr/local/bin/ayni" "$scratch/ayni" || return 1
  docker rm "$container" >/dev/null || return 1
  container=''
  [[ "$(sha256sum "$cli" | cut -d' ' -f1)" == "$(sha256sum "$scratch/ayni" | cut -d' ' -f1)" ]] || return 1
  # Ayni's entrypoint owns certificate, protected inventory and lock validation.
  "${runtime[@]}" "$image" /bin/true
}

reference="ghcr.io/$repository:ayni-${lock#sha256:}"
if docker pull --platform "$platform" "$reference" && validate_image "$reference"; then
  source=cache
else
  echo 'Managed image unavailable or incompatible; building the locked environment.'
  if [[ -n "$container" ]]; then docker rm "$container" >/dev/null; container=''; fi
  build=("$cli" env build --repo-root .)
  if [[ -n "${AYNI_CI_EXECUTOR:-}" ]]; then build+=(--executor-image "$AYNI_CI_EXECUTOR"); fi
  "${build[@]}"
  validate_image "$(jq -er '.image_id' .ayni/environment/build.json)"
  source=build
fi
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  printf 'image=%s\nsource=%s\n' "$image" "$source" >> "$GITHUB_OUTPUT"
fi

# A quality failure is final: retain the evidence and never rebuild afterwards.
set +e
"${runtime[@]}" "$image" /usr/local/bin/ayni check \
  --config "$(jq -er '.repository.contract_path' .ayni.lock)" --output markdown 2>&1 | tee "$report"
status=${PIPESTATUS[0]}
set -e
jq -e --arg lock "$lock" --argjson targets "$(jq '.targets | length' .ayni.lock)" '
  .execution_mode == "managed" and .environment_lock_fingerprint == $lock and
  .completion.scope == "repository" and .completion.state == "complete" and
  .completion.expected_targets == $targets and .completion.detected_targets == $targets and
  .completion.completed_targets == $targets and .completion.skipped_targets == 0 and
  .completion.issues == []' .ayni/last/signals.json >/dev/null
exit "$status"
