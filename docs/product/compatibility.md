# Runtime and tool compatibility

Ayni records exact versions in a repository environment lock. Qualification is
the separate evidence that those versions work together for a documented project
shape and platform. A version pin alone does not establish that compatibility.

## Current guarantees

- Managed environments resolve exact runtime, package-manager and signal-tool
  versions from adapter requirements and native project metadata.
- Rust and Go isolated signal tools use adapter-pinned baselines. Node, Python
  and Kotlin project tools retain the versions selected by native declarations
  and locks.
- Reconciliation checks declarations and resolutions. It does not universally
  require project-selected tools to equal adapter baselines.
- Host collection uses available native tools without enforcing the managed
  version contract. Host and managed artifacts remain distinct.

The [adapter capability matrix](capabilities.md) describes signal availability.
Each language guide describes supported project shapes and current version
selection. Neither establishes a universally enforced matrix of exact runtime
and tool combinations.

## Planned support policy

The agreed direction is to admit explicitly qualified exact combinations.
Runtime and tool compatibility enforcement remains planned. There is no new
configuration setting or command that enables this policy today.

A qualified combination must identify:

| Input | Required qualification detail |
| --- | --- |
| Ayni | Exact release or source revision and relevant CLI/artifact contracts |
| Language | Exact runtime/compiler and toolchain components |
| Package manager | Exact version and native lock format |
| Signal tools | Exact versions, provider alternatives and report interfaces |
| Project | Supported workspace shape and required native configuration |
| Platform | Operating system, architecture and managed execution profile |

Qualifying each tool separately does not qualify every combination of those
tools. A repository lock records one concrete environment; the planned support
catalog will state which combinations Ayni has qualified.

### Admission and upgrades

The planned managed admission boundary rejects combinations outside the qualified
catalog with an actionable explanation. It must not silently downgrade tools,
refresh native locks or select an alternative environment. Exact behavior for
host diagnostics will be defined with enforcement implementation; current host
execution remains unchanged.

Adding or upgrading a combination requires reviewed compatibility evidence,
updated adapter inventories and fixtures, and explicit repository lock refreshes.
Quality commands continue to use existing inputs without upgrading them.
Support retirement must identify the affected combination, the release ending
support and the qualified replacement before removing its compatibility code
and tests. No automatic support period or version-range promise is established.

## Qualification evidence

| Evidence | What it establishes | What remains unproven |
| --- | --- | --- |
| Native declarations and inventory tests | Expected versions and catalog coverage agree | Tool execution and environment readiness |
| Captured report/parser fixtures | The collector understands a report shape | Fresh execution on the declared runtime |
| Real tool execution | The tool runs and produces the expected measurement for the fixture | Managed image preparation and platform compatibility |
| Managed environment execution | The exact image can prepare and collect complete evidence for the recorded project and platform | Other combinations, project shapes or deployment profiles |

The existing [baseline fixtures](../contributing/tooling-baseline-fixtures.md)
record limited execution evidence, including Python mutation and Kotlin
coverage/mutation fixtures. Their runtime descriptions include major/minor
selectors and do not establish a complete exact-version managed support catalog.
No full cross-language catalog is claimed by this page.

## Native configuration and overrides

Projects own native test, coverage, lint and build configuration. Adapters inspect
that configuration and document the prerequisites they understand. Ayni owns its
quality policy, adapter integrations and managed environment contracts.

Reconciliation is read-only. Use the native package manager to repair project
metadata, then explicitly refresh and build the Ayni lock. General-purpose
rewriting of native configuration is outside the current contract.

Custom command overrides exclude default tools from reconciliation for that
signal. A normalized result from an override does not establish qualification
of the default integration or of arbitrary replacement tools.

## Related contracts

- [Signal-tool inspection](../contributing/tooling-reconciliation.md) explains current version authority and diagnostics.
- [Managed environments](environments.md) defines exact locking, construction and launch.
- [Security and trust](security.md) separates environment verification from quality and trusted execution.
