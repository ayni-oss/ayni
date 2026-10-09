# Ayni

Ayni is a local quality protocol for repositories that use AI agents.
Maintainers commit policy, Ayni runs repository tools in the current workspace,
and agents receive scoped, actionable evidence. Ayni can also build a portable
development image containing the repository's locked tools.

Ayni is intentionally forge-neutral and deterministic. It does not generate
probabilistic review commentary; it normalizes real repository-tool outcomes
into evidence that humans and AI agents can reproduce.

**Documentation:** [Installation](docs/getting-started/installation.md) ·
[Quickstart](docs/getting-started/quickstart.md) ·
[How Ayni works](docs/getting-started/how-ayni-works.md) ·
[Security model](docs/product/security.md)

## Why

AI agents increase delivery speed, but they also increase change volume beyond
what humans can reliably review line by line.

Ayni shifts quality control from "inspect every diff" to "measure local behavior
and structural health". Repositories define explicit expectations in
`.ayni.toml`; agents respond to those expectations with measurable repairs.

Ayni does not generate code or replace your test and analysis tools. It
orchestrates them, normalizes their output, and turns failures into structured
repair targets.

## What Ayni Does

- can create or update its marked agent-facing guidance in `AGENTS.md` with an explicit command
- defines the repository-agent quality contract in `.ayni.toml`
- models `test`, `coverage`, `size`, `complexity`, `deps`, and `mutation` evidence, with explicit adapter capability tiers
- runs language-specific tooling through adapters in the current execution environment
- writes machine-readable artifacts under `.ayni/`
- prints terminal or Markdown reports for local workflows and AI repair loops

## Install

Use the [installation guide](docs/getting-started/installation.md) for verified release archives and the installer.
Published versions are listed on [GitHub Releases](https://github.com/ayni-oss/ayni/releases).

### From source

Build from this repository with its pinned Rust toolchain:

```sh
cargo install --locked --path cli
```

## Quick Start

Ayni keeps the repository's quality policy separate from its execution
environment:

```text
.ayni.toml + .ayni.lock / OCI image → check / verify / impact
 policy          exact runtime          measured evidence
```

Preview a minimal test-only policy before writing anything, create it
explicitly, then inspect the validated contract:

```sh
ayni init --dry-run
ayni init --write
ayni contract show
```

Build a reusable environment image when developers, CI, or a coding platform
need the locked toolchain. Signing is optional: supply both signing variables
for a builder-identity signature, or neither for local-consistency metadata.

```sh
ayni env show
ayni env lock
ayni env build
ayni env doctor
ayni check
```

Inspect managed-environment storage before reclaiming it. Pruning is a dry run
by default; `--current` explicitly resets the current repository's derived
state, which Ayni recreates from the lock on the next managed command:

```sh
ayni env storage
ayni env prune
ayni env prune --apply --current
```

`check`, `verify`, and `impact run` always run in the current workspace. On a
clean host they report missing tools with setup guidance. Inside a built Ayni
image they verify `/etc/ayni/runtime.json` against the attached checkout lock
before activating locked tools. Supported project shapes and the complete
lifecycle are documented in the [quickstart](docs/getting-started/quickstart.md)
and [environment guide](docs/product/environments.md).

Use focused verification for the inner repair loop:

```sh
ayni verify test --language rust --package my-crate --name test_filter
ayni verify coverage --language node --root apps/web
```

Focused evidence is written to `.ayni/verify/last/signals.json` and never
replaces the repository result at `.ayni/last/signals.json`. Human and Markdown
reports omit rerun commands; those commands remain structured finding evidence
in the JSON artifact. Print the exact, deduplicated commands explicitly with:

```sh
ayni verify list
ayni verify list --artifact .ayni/verify/last/signals.json
```

Copy these artifact-supplied commands rather than broadening them by hand.
Signal availability and selector support are adapter-specific and documented
in the [capability matrix](docs/product/capabilities.md) and adapter guides.

Use impact planning for a conservative change-scoped loop against an explicit
Git base:

```sh
ayni impact show --base main
ayni impact run --base main
```

Impact evidence is change-scoped and never claims repository completion; run an
unscoped `ayni check` at the caller's completion boundary. See the
[impact contract](docs/product/impact.md) for workspace mapping and uncertainty
rules.

Run `ayni agents sync` explicitly when you want the managed guidance block
created or refreshed. It points agents to the repository policy and exact
finding commands without replacing repository-specific instructions.

Generate Markdown or current schema-v4 JSON output:

```sh
ayni check --output markdown
ayni check --output json
```

Compare two already-produced complete current-schema artifacts explicitly:

```sh
ayni results compare --baseline before.json --candidate after.json
```

Built images also support `check`, `verify`, and `impact run` when launched
externally with the [portable certified execution contract](docs/product/environments.md#portable-certified-execution), without an OCI engine inside the container.

For command details, image launch examples, output behavior, and result
comparison semantics, see the [CLI reference](docs/cli.md),
[configuration reference](docs/product/config.md), and
[conceptual guide](docs/getting-started/how-ayni-works.md).

## Builder, environment, and quality evidence

The versioned `ayni-builder` factory produces a separate repository development image.
The factory contains build clients; the resulting image contains the locked tools and prepared dependencies used during development.
Quality commands run in the current workspace and never launch or build a container.

Environment certification binds metadata and protected content to the lock and optional trusted signer.
It does not certify editable source quality. Quality evidence measures the configured policy, tools, and tests;
a passing result does not grant merge approval or establish regulatory compliance.
See the [security model](docs/product/security.md) for verification boundaries.

## Signals

Ayni emits a closed signal vocabulary shared across language adapters.

| Signal | Purpose |
| --- | --- |
| `test` | Test execution health and failures |
| `coverage` | Coverage depth and weak or uncovered areas |
| `size` | Module and file growth against line-count budgets |
| `complexity` | Function-level complexity against thresholds |
| `deps` | Forbidden architectural dependency edges |
| `mutation` | Test effectiveness against simulated behavioral change |

For the canonical vocabulary and version selection, see
[`docs/product/signals.md`](docs/product/signals.md); the current self-contained
JSON envelope is [schema v4](docs/product/signals/v4.md). Availability is
adapter-specific: see the [capability tiers](docs/product/capabilities.md).
Historical schemas remain archived but are not part of primary navigation.

## Configuration

`.ayni.toml` is the handoff point between humans and agents: which languages and
roots are in scope, which signals run, and which limits define healthy code.

```toml
[checks]
test = true
coverage = true
size = true
complexity = true
deps = true
mutation = false

[languages]
enabled = ["rust"]

[rust.size]
"*.rs" = { warn = 400, fail = 700, exclude = ["target/**", ".ayni/**"] }

[rust.complexity]
fn_cyclomatic = { warn = 10, fail = 20 }
```

For the full configuration reference, see
[`docs/product/config.md`](docs/product/config.md).

Size and complexity are maximums: at `warn` or higher they warn, and at `fail`
or higher they fail. Coverage is a minimum: below `warn` it warns and below
`fail` it fails. Thus `{ warn = 400, fail = 700 }` warns at 400 lines and fails
at 700, while `{ warn = 80, fail = 70 }` warns below 80% coverage and fails
below 70%. Warnings remain visible but do not make a row fail.

## How It Fits Together

The platform keeps product semantics, language integrations, and CLI behavior
separate:

```text
core  <-  adapters/common  <-  adapters/<language>  <-  cli
core  <-  adapters/common  <-  environment          <-  cli
```

Language adapters own ecosystem-specific discovery, tool invocation, impact
mapping, and normalization. Core owns typed product contracts. The environment
backend consumes validated locks and preparation plans without interpreting
language manifests. The CLI owns orchestration and output.

For layer boundaries and change rules, see [ARCHITECTURE.md](ARCHITECTURE.md).

## Documentation

- [CLI reference](docs/cli.md)
- [Configuration reference](docs/product/config.md)
- [Managed environments](docs/product/environments.md)
- [Security and trust model](docs/product/security.md) and [vulnerability reporting policy](SECURITY.md)
- [Signal contract index](docs/product/signals.md) and [current schema v4](docs/product/signals/v4.md)
- [Adapter capability tiers](docs/product/capabilities.md)
- [Runtime and setup rules](docs/product/runtime.md)
- [Architecture](ARCHITECTURE.md)
- Language adapters:
  [Rust](docs/adapters/rust.md),
  [Go](docs/adapters/go.md),
  [Node](docs/adapters/node.md),
  [Python](docs/adapters/python.md),
  [Kotlin](docs/adapters/kotlin.md)

## Contributing

Developer workflow, architecture constraints, and repository checks live in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## License

Ayni is licensed under the Apache License, Version 2.0 (`Apache-2.0`). See
[`LICENSE`](LICENSE) for the full license text.
