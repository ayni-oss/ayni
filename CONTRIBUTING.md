# Contributing

Thanks for helping improve Ayni.

## Code of conduct and governance

Participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md),
[Community guide](COMMUNITY.md), and [Governance](GOVERNANCE.md). Use public
issues and pull requests for ordinary project work; follow
[SECURITY.md](SECURITY.md) for vulnerabilities.

## Scope

The open-source CLI supports:

- managed and explicit host execution for `check`, focused `verify`, and
  `impact run`
- the lock-driven `env show`, `env lock`, `env doctor`, `env build`, `env shell`,
  and `env run` lifecycle
- `contract show` and `results compare`
- `agents sync` to create or refresh only Ayni's marked `AGENTS.md` section

Out of scope:

- hosted service workflows and external run storage
- forge-specific GitHub, GitLab, or Bitbucket integration
- implicit remote Git fetches or baseline selection

## Development

Run the standard checks from the repository root:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo check --workspace --all-features
```

During an implementation, exercise the checkout binary and use the narrowest
supported focused verification command. The adapter documentation records the
honest selector matrix; unsupported selectors fail before tool invocation.

```sh
cargo run -p ayni-cli -- verify test --language rust --package ayni-cli
```

Do not use focused evidence as repository completion evidence. An unscoped
`cargo run -p ayni-cli -- check --config ./.ayni.toml` is the repository gate
and sole writer of `.ayni/last/signals.json`; use `--host` only as the explicit
escape hatch. Focused runs write only `.ayni/verify/last/signals.json`. Re-run the exact
`verification.command` attached to a finding when one is available.

## Documentation

The docs site lives under `docs/` and uses the root npm scripts:

```sh
npm install
npm run docs:dev
npm run docs:build
npm run docs:preview
```

Use `npm ci` instead of `npm install` when you want a clean, lockfile-driven install.

Regenerate the CLI reference after changing commands or flags:

```sh
cargo doc-cli > docs/cli.md
```

Every pull request verifies that `docs/cli.md` matches `cargo doc-cli` and that
VitePress builds successfully as part of the event-driven `ayni-status` gate.
Pushes to `main` additionally deploy the uploaded `docs/.vitepress/dist` artifact
to GitHub Pages; source documentation remains under `docs/`.

Documentation is deployed only when the organization configures GitHub Pages for this repository.

For language adapter implementation guidance, see
[`docs/contributing/adapters.md`](docs/contributing/adapters.md).

## Architecture

- CLI handles arguments, orchestration, and local output.
- Core owns policy, signal, environment-plan, lock, impact, and adapter contracts.
- `adapters/common` owns shared safe execution and filesystem plumbing.
- Language adapters own ecosystem-specific discovery, version resolution,
  dependency preparation, tool execution, and impact mapping.
- The environment crate consumes validated locks and preparation plans; it does
  not interpret language manifests.
- Dependencies point inward along the flows documented in `ARCHITECTURE.md`.

## Pull Request Checklist

- PR title follows `<type>(<scope>): <description>`; the title becomes the
  squash-merge commit and drives the automated changelog and release version.
- Trusted metadata automation maps `feat` and `perf` to `kind/feature`, `fix` to
  `kind/bug`, `docs` or a `docs` scope to `kind/documentation`, a `security`
  scope to `kind/security`, and other supported types to `kind/chore`. It owns
  those five labels on pull requests but does not infer priority.
- Tests added or updated when behavior changes.
- No managed service dependency introduced.
- Local artifact behavior preserved.
- Repository completion uses only unscoped `check`; focused or impact evidence
  has not replaced `.ayni/last/signals.json`.
- README or docs updated if behavior changed.
- `ayni agents sync` is idempotent and preserves user content outside Ayni's marked block.
- `cargo fmt`, `cargo clippy`, `cargo test`, and `cargo check` pass.

## Releases

Release metadata is managed by [Release Please](https://github.com/googleapis/release-please)
from the Conventional Commit title of each squash-merged pull request. It
opens a release pull request that updates the version and changelog. Merging
that release pull request creates the GitHub release and the `ayni-vX.Y.Z`
tag; the release workflow then builds, signs, checksums, and validates the
published CLI artifacts.

The project follows Semantic Versioning:

- `feat` and other backwards-compatible functionality normally produce a
  minor release.
- `fix`, documentation, maintenance, and security fixes normally produce a
  patch release.
- Breaking changes use `!` (for example `feat!:` or `feat(cli)!:`) and require
  a major release.

The `ayni-` tag prefix is retained because it is part of the existing installer,
archive, recovery, and changelog contracts. It is not a second version number;
the semantic version remains `X.Y.Z` and the CLI displays that unprefixed
version. Do not create tags or GitHub releases manually except through the
documented recovery workflow.

## Developer Certificate of Origin and signatures

Every contribution, including maintainer contributions, must carry a Developer
Certificate of Origin sign-off:

```sh
git commit -s
```

Maintainers also cryptographically sign commits. A DCO sign-off and a
cryptographic signature serve different purposes; both are required for
maintainer changes.

## Licensing

By contributing to Ayni, you agree that your contribution is licensed under
the same license as the project: Apache License, Version 2.0 (`Apache-2.0`).
