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
