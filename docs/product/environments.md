# Development environments

Ayni has two separate responsibilities. `ayni check`, `ayni verify`, and
`ayni impact run` run in the current checkout using its current execution
environment. `ayni env build` produces a reusable OCI development image.

Quality commands never build, launch, or re-execute a container. On a host,
missing tools produce setup guidance. In an Ayni image, `/etc/ayni/runtime.json`
is verified against the attached checkout's lock and protected tool inventory
before locked tool activation is applied.

## Lifecycle

```sh
ayni env show
ayni env lock
ayni env build
```

`env lock` is explicit and writes the committed `.ayni.lock`. `env build`
requires that current lock and never refreshes it. `env doctor`, `env storage`,
and `env prune` diagnose and maintain existing environment state. Use ordinary
container or platform tooling to start the image.

Builds do not push images. `--tag` adds a local image tag. With neither signing
variable set, Ayni writes unsigned metadata that establishes local consistency.
With both `AYNI_ENV_CERTIFICATE_SIGNING_KEY` and
`AYNI_ENV_CERTIFICATE_KEY_ID` set, Ayni signs the same metadata and verifies the
configured repository trust key. Supplying only one signing value fails.

## Signing an environment image

Use an Ed25519 key dedicated to environment-image signing. Ayni expects the
private value as a **32-byte seed encoded as 64 lowercase hexadecimal
characters**; it does not accept a PEM private key. Keep that seed only in a
trusted secret store or trusted CI environment.

The following commands generate a seed with OpenSSL, derive its public key,
and print both values. Run them on a trusted machine and transfer the private
seed to the secret store without placing it in the repository:

```sh
private_key_file="$(mktemp)"
trap 'rm -f "$private_key_file"' EXIT

openssl genpkey -algorithm ED25519 -out "$private_key_file"
signing_seed="$(openssl pkey -in "$private_key_file" -outform DER | tail -c 32 | xxd -p -c 64)"
public_key="$(openssl pkey -in "$private_key_file" -pubout -outform DER | tail -c 32 | xxd -p -c 64)"

printf 'signing seed: %s\npublic key: %s\n' "$signing_seed" "$public_key"
```

Choose a stable key identifier and pin the public key in `.ayni.toml`:

```toml
[environment.certificate.trusted_keys]
release-2026 = "<64-character lowercase Ed25519 public key>"
```

Commit that policy change and refresh the lock:

```sh
ayni env lock
```

For a trusted build, set these two values together:

```sh
export AYNI_ENV_CERTIFICATE_SIGNING_KEY='<64-character lowercase signing seed>'
export AYNI_ENV_CERTIFICATE_KEY_ID='release-2026'
ayni env build
```

The key identifier must use only ASCII letters, numbers, `.`, `-`, or `_`.
`ayni env build` fails before building when the seed is malformed, only one
variable is set, or the derived public key does not match the pinned key.
Use neither variable for an unsigned build.

## Launching an image

The image defaults to `/bin/sh`. A platform attaches the checkout at its chosen
working directory, then invokes native development tools or plain Ayni commands:

```sh
docker run --rm -it \
  --mount type=bind,source="$PWD",target=/workspace \
  --workdir /workspace \
  <image> /bin/sh

ayni check
```

The platform owns networking, user identity, resource limits, and isolation.
Ayni records a verified environment context in its result artifact when runtime
metadata is present; it does not claim that the launcher isolated the workload.
