# Configuration

`.ayni.toml` defines repository signals, language roots, thresholds, and
environment inputs. `.ayni.lock` records the resolved tools and dependency
preparation inputs. Commit both files.

Use `ayni contract show` to inspect the effective quality contract and `ayni
env show` to inspect the environment plan. Run `ayni env lock` intentionally
after changing environment inputs; neither `check` nor `env build` updates it.

## Quality commands

`ayni check` evaluates every configured target. `ayni verify <signal>` runs a
focused signal, and `ayni impact run --base <revision>` runs the checks selected
by an explicit Git change. All execute in the current workspace and execution
environment. Missing local tools produce setup failures; a verified image
supplies the locked tools.

Each quality command replaces its prior evidence before it starts, so an old
artifact cannot be mistaken for the current result. `check` writes
`.ayni/last/signals.json`; focused verification writes
`.ayni/verify/last/signals.json`.

## Environment inputs

Use `[environment.tools]` for repository-wide pinned tools and
`[environment.debian]` for pinned Debian packages. Adapter-owned runtime,
package-manager, and dependency metadata is discovered from the configured
language roots. Container launch policy, network access, Docker sockets, and
resource limits belong to the platform running the image, not this file.

After changing an environment input, run:

```sh
ayni env lock
ayni env build
```

`env build` requires a current lock and never pushes an image. Use registry and
container tooling to publish and launch the resulting image.
