# Shared fake OCI executor protocol; test delegates own preparation and failures.
if [ -n "${AYNI_ENV_CERTIFICATE_SIGNING_KEY:-}" ] || [ -n "${AYNI_ENV_CERTIFICATE_KEY_ID:-}" ]; then
  echo 'certificate signing environment leaked to OCI client' >&2
  exit 97
fi
engine_dir=$(dirname "$0")
state="$engine_dir/executor-image.json"
labels_state="$engine_dir/executor-image.labels.json"
id_state="$engine_dir/executor-image.id"
tag_state="$engine_dir/executor-image.tag"
certificate_state="$engine_dir/executor-certificate.json"
manifest_state="$engine_dir/executor-protected-content.manifest"
candidate_labels="$engine_dir/executor-candidate.labels.json"
candidate_id_state="$engine_dir/executor-candidate.id"
candidate_certificate="$engine_dir/executor-candidate-certificate.json"
candidate_manifest="$engine_dir/executor-candidate-protected-content.manifest"
base_dockerfile="$engine_dir/executor-assembled.Dockerfile"
certified_dockerfile="$engine_dir/executor-certified.Dockerfile"
previous_state="$engine_dir/executor-previous-image.json"
previous_labels="$engine_dir/executor-previous-image.labels.json"
previous_id="$engine_dir/executor-previous-image.id"
previous_tag="$engine_dir/executor-previous-image.tag"
previous_certificate="$engine_dir/executor-previous-certificate.json"
previous_manifest="$engine_dir/executor-previous-protected-content.manifest"

write_state() {
  image_id=$1
  image_tag=$2
  printf '%s' "$image_id" > "$id_state"
  printf '%s' "$image_tag" > "$tag_state"
  printf '[{"Id":"%s","RepoTags":["%s"],"Config":{"Labels":' "$image_id" "$image_tag" > "$state"
  cat "$labels_state" >> "$state"
  printf '}}]\n' >> "$state"
}

case "$1" in
  buildx) printf '{"digest":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}\n'; exit 0;;
  pull) for argument in "$@"; do case "$argument" in registry.example/missing*) echo "executor unavailable" >&2; exit 1;; esac; done; exit 0;;
esac
last=''
penultimate=''
entrypoint=''
previous=''
for argument in "$@"; do
  [ "$previous" != --entrypoint ] || entrypoint=$argument
  penultimate=$last
  last=$argument
  previous=$argument
done
case "$1:$2:$last" in
  image:inspect:*@sha256:*)
    printf '[{"Id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Os":"linux","Architecture":"%s","Config":{"Labels":{"org.opencontainers.image.revision":"ffffffffffffffffffffffffffffffffffffffff","dev.ayni.executor.lock-schema":"0.8.0","dev.ayni.executor.recipe":"%s","dev.ayni.provisioning.schema":"1","dev.ayni.environment.variant":"debian","dev.ayni.environment.mise-version":"2025.2.4"}}}]\n' "$ARCH" "${AYNI_TEST_EXECUTOR_RECIPE:-1}"
    exit 0;;
esac

if [ "$1" = run ]; then
  case "$entrypoint" in
    sha256sum|/usr/bin/sha256sum) echo 'eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee  /usr/local/bin/ayni'; exit 0;;
    /usr/local/bin/ayni) if [ "$last" = --version ]; then echo "ayni $VERSION"; exit 0; fi;;
    cat|/usr/bin/cat)
      use_candidate=0
      if [ -f "$candidate_id_state" ] && [ "$penultimate" = "$(cat "$candidate_id_state")" ]; then use_candidate=1; fi
      case "$last:$use_candidate" in
        /etc/ayni/runtime.json:1) cat "$candidate_certificate"; exit $?;;
        /etc/ayni/protected-content.manifest:1) cat "$candidate_manifest"; exit $?;;
        /etc/ayni/runtime.json:0) cat "$certificate_state"; exit $?;;
        /etc/ayni/protected-content.manifest:0) cat "$manifest_state"; exit $?;;
      esac;;
    /bin/sh)
      for argument in "$@"; do
        case "$argument" in
          *"sha256sum --zero"*)
            [ "${AYNI_TEST_MANIFEST_FAILURE:-0}" != 1 ] || { echo 'manifest generation failed' >&2; exit 9; }
            printf '%s  /etc/ayni/mise.toml\0' '1111111111111111111111111111111111111111111111111111111111111111'
            printf '%s  /opt/ayni/mise/installs/rust/bin/rustc\0' '2222222222222222222222222222222222222222222222222222222222222222'
            printf '%s  /usr/local/bin/ayni\0' '3333333333333333333333333333333333333333333333333333333333333333'
            printf '%s  /usr/local/bin/mise\0' '4444444444444444444444444444444444444444444444444444444444444444'
            exit 0;;
          *"type l"*)
            printf '/opt/ayni/mise/shims/rustc\0../bin/mise\0'
            exit 0;;
          *"stat -c"*)
            if [ "${AYNI_TEST_CERTIFICATE_OWNER_FAILURE:-0}" = 1 ]; then
              printf '10001:10001:444\n0:0:444\n'
            elif [ "${AYNI_TEST_CERTIFICATE_MODE_FAILURE:-0}" = 1 ]; then
              printf '0:0:444\n0:0:600\n'
            else
              printf '0:0:444\n0:0:444\n'
            fi
            exit 0;;
        esac
      done;;
  esac
fi

if [ "$1" = image ] && [ "$2" = inspect ]; then
  if [ -f "$candidate_id_state" ] && [ "$last" = "$(cat "$candidate_id_state")" ]; then
    case "$*" in
      *"{{.Id}}"*) cat "$candidate_id_state"; printf '\n';;
      *"--format"*) cat "$candidate_labels";;
      *) exit 1;;
    esac
    exit $?
  fi
  if [ -f "$state" ] && { [ "$last" = "$(cat "$id_state")" ] || [ "$last" = "$(cat "$tag_state")" ]; }; then
    case "$*" in
      *"{{.Id}}"*) cat "$id_state"; printf '\n';;
      *"--format"*) cat "$labels_state";;
      *) cat "$state";;
    esac
    exit $?
  fi
  case "$last:$*" in ayni-env:*:*"{{.Id}}"*) exit 1;; esac
fi

if [ "$1" = image ] && [ "$2" = tag ]; then
  if [ -f "$candidate_id_state" ] && [ "$3" = "$(cat "$candidate_id_state")" ]; then
    if [ -f "$state" ]; then
      cp "$state" "$previous_state" || exit $?
      cp "$labels_state" "$previous_labels" || exit $?
      cp "$id_state" "$previous_id" || exit $?
      cp "$tag_state" "$previous_tag" || exit $?
      cp "$certificate_state" "$previous_certificate" || exit $?
      cp "$manifest_state" "$previous_manifest" || exit $?
    fi
    cp "$candidate_labels" "$labels_state" || exit $?
    cp "$candidate_certificate" "$certificate_state" || exit $?
    cp "$candidate_manifest" "$manifest_state" || exit $?
    write_state "$3" "$4"
    exit 0
  fi
  if [ -f "$previous_id" ] && [ "$3" = "$(cat "$previous_id")" ]; then
    cp "$previous_state" "$state" || exit $?
    cp "$previous_labels" "$labels_state" || exit $?
    cp "$previous_id" "$id_state" || exit $?
    cp "$previous_tag" "$tag_state" || exit $?
    cp "$previous_certificate" "$certificate_state" || exit $?
    cp "$previous_manifest" "$manifest_state" || exit $?
    exit 0
  fi
fi

if [ "$1" = image ] && [ "$2" = rm ] && [ -f "$tag_state" ] && [ "$3" = "$(cat "$tag_state")" ]; then
  rm -f "$state" "$labels_state" "$id_state" "$tag_state" "$certificate_state" "$manifest_state"
  exit 0
fi

if [ "$1" = build ]; then
  previous=''
  file=''
  iidfile=''
  tag=''
  context=''
  for argument in "$@"; do
    case "$previous" in --file) file=$argument;; --tag) tag=$argument;; --iidfile) iidfile=$argument;; esac
    previous=$argument
    context=$argument
  done
  case "$file" in
    *Certified.Dockerfile)
      [ "${AYNI_TEST_CERTIFIED_BUILD_FAILURE:-0}" != 1 ] || { echo 'certificate installation failed' >&2; exit 9; }
      cp "$context/certificate.json" "$candidate_certificate" || exit $?
      cp "$context/protected-content.manifest" "$candidate_manifest" || exit $?
      cp "$file" "$certified_dockerfile" || exit $?
      sources="$base_dockerfile $file"
      ;;
    *)
      "$engine_dir/docker-delegate" "$@" || exit $?
      cp "$file" "$base_dockerfile" || exit $?
      sources="$file"
      ;;
  esac
  if command -v sha256sum >/dev/null 2>&1; then digest=$(cat $sources 2>/dev/null | sha256sum); else digest=$(cat $sources 2>/dev/null | shasum -a 256); fi
  digest=${digest%% *}
  image_id="sha256:$digest"
  [ -z "$iidfile" ] || printf '%s\n' "$image_id" > "$iidfile"

  case "$file" in
    *Certified.Dockerfile)
      printf '%s' "$image_id" > "$candidate_id_state"
      printf '{' > "$candidate_labels"
      separator=''
      for label in owner schema lock-fingerprint base-digest ayni-version mise-version platform preparation-digest executor recipe certificate-schema certificate-key-id protected-content-root; do
        value=$(cat $sources 2>/dev/null | sed -n "s/.*dev.ayni.environment.$label=\"\([^\"]*\)\".*/\1/p" | tail -n 1)
        [ -n "$value" ] || continue
        printf '%s"dev.ayni.environment.%s":"%s"' "$separator" "$label" "$value" >> "$candidate_labels"
        separator=','
      done
      printf '}' >> "$candidate_labels"
      ;;
  esac
  exit 0
fi

"$engine_dir/docker-delegate" "$@"
result=$?
if [ "$result" = 0 ] && [ "$1" = run ] && [ "${AYNI_TEST_OMIT_ARTIFACT:-0}" != 1 ]; then
  for argument in "$@"; do
    if [ "$argument" = check ]; then
      mkdir -p "$engine_dir/../.ayni/last"
      printf '{"schema_version":"0.4.0","fixture":"engine-protocol"}\n' > "$engine_dir/../.ayni/last/signals.json"
    fi
  done
fi
exit "$result"
