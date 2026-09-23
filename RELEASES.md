# Releases

Ayni is pre-1.0. Releases are made from `main` when a coherent, validated set
of changes is ready; the project does not promise a calendar schedule.

Stable releases use tags in the form `ayni-vX.Y.Z`. A security release may be
made out of cycle. The latest stable release line is supported; older lines and
development snapshots do not receive guaranteed backports.

Release Please creates the release pull request from Conventional Commit
titles on squash-merged pull requests. The title becomes the commit subject on
`main`, so every pull request must use `<type>(<scope>): <description>`; CI
checks this before merge. Release Please updates the version and changelog,
and merging its release pull request creates the tag and GitHub release.

Use SemVer increments as follows: backwards-compatible features normally bump
the minor version, fixes and maintenance normally bump the patch version, and
breaking changes use `!` in the title and bump the major version. The
`ayni-` tag prefix is retained as part of the existing installer, archive,
recovery, and changelog contracts; the semantic version itself remains
`X.Y.Z`.

Each release is expected to include signed source provenance, checksums, and
platform archives containing the `ayni` binary and `LICENSE`. OCI environment
images are published by digest and verified through the documented workflow.
Before the first release is published from `ayni-oss`, users should build from
source rather than rely on release-install commands.

Release decisions, version changes, and any support-policy changes are recorded
in public pull requests under [GOVERNANCE.md](GOVERNANCE.md).
