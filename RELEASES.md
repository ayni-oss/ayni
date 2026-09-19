# Releases

Ayni is pre-1.0. Releases are made from `main` when a coherent, validated set
of changes is ready; the project does not promise a calendar schedule.

Stable releases use tags in the form `ayni-vX.Y.Z`. A security release may be
made out of cycle. The latest stable release line is supported; older lines and
development snapshots do not receive guaranteed backports.

Each release is expected to include signed source provenance, checksums, and
platform archives containing the `ayni` binary and `LICENSE`. OCI environment
images are published by digest and verified through the documented workflow.
Before the first release is published from `ayni-oss`, users should build from
source rather than rely on release-install commands.

Release decisions, version changes, and any support-policy changes are recorded
in public pull requests under [GOVERNANCE.md](GOVERNANCE.md).
