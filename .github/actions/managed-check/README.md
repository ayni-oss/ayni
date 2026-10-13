# Managed repository checks

This composite action checks the caller's checkout using its committed
`.ayni.lock`. Supply an installed Linux Ayni executable, or the exact checkout
binary and immutable executor image that the workflow built. Pin external uses
to a reviewed full commit SHA:

```yaml
- uses: ayni-oss/ayni/.github/actions/managed-check@<full commit SHA>
  with:
    cli: /path/to/ayni
    repository: ${{ github.event.pull_request.base.repo.full_name || github.repository }}
```

The base repository determines the read-only cache lookup:
`ghcr.io/<owner>/<repository>:ayni-<full lock fingerprint without sha256:>`.
The action pulls the selected platform, freezes the resulting Docker image ID,
and compares its lock/platform labels and Ayni executable bytes. A normal
entrypoint probe verifies the runtime certificate, protected inventory and
current lock, then restores prepared dependencies. Missing or rejected cache
images trigger `ayni env build --repo-root .`, with the supplied
`executor-image` when present. No image is published and no registry login is
performed.

The quality check uses that same immutable image and entrypoint. It requires
managed evidence matching the lock, with complete repository target accounting.
A failed check never triggers another
build. Previous `.ayni/last` artifacts are removed first; current JSON evidence
and the Markdown report remain available even when the action fails. Upload
both from the calling workflow with `if: ${{ !cancelled() }}`.

Inputs are `cli`, `repository`, optional `executor-image`, `platform`
(`linux/amd64` by default), and `report-path` (`runner.temp/ayni.md` by default).
Outputs are `image` (immutable Docker ID), `source` (`cache` or `build`), and
`report-path`. The runner needs Bash, jq, Docker/Buildx, and permission to
launch containers and write generated files to its checkout. The action keeps
the image's certified entrypoint and supplies writable runtime cache paths.

Run its launcher tests with:

```sh
python3 -m unittest discover -s .github/actions/managed-check -p 'test_*.py'
```
