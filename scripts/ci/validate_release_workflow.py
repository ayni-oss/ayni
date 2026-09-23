"""Validate the intentionally small, source-bound release workflow."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
JOB = re.compile(r"^  ([a-z][a-z0-9-]*):\n", re.MULTILINE)


def job_block(source: str, name: str) -> str:
    matches = list(JOB.finditer(source))
    for index, match in enumerate(matches):
        if match.group(1) == name:
            end = matches[index + 1].start() if index + 1 < len(matches) else len(source)
            return source[match.start() : end]
    raise ValueError(f"missing release workflow job: {name}")


def require(errors: list[str], condition: bool, message: str) -> None:
    if not condition:
        errors.append(message)


def main() -> int:
    source = WORKFLOW.read_text()
    publication = (WORKFLOW.parent / "release-publication.yml").read_text()
    errors: list[str] = []

    try:
        release = job_block(source, "release")
        caller = job_block(source, "publication")
        build = job_block(publication, "build")
        publish = job_block(publication, "publish")
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1

    for removed_job in ("sync-release-lock", "release-completion"):
        require(errors, f"  {removed_job}:\n" not in source,
                f"release workflow must not retain {removed_job}")
    for removed_job in (
        "environment-image", "environment-manifest", "release-assets",
        "release-smoke", "environment-image-smoke", "fixture-plan",
        "release-validation", "release-completion",
    ):
        require(errors, f"  {removed_job}:\n" not in publication,
                f"binary publication must not retain {removed_job}")

    require(errors,
            'WORKFLOW_REF: ${{ github.ref }}' in release
            and 'test "$WORKFLOW_REF" = "refs/heads/main"' in release,
            "manual recovery must validate the main workflow revision")
    require(errors,
            "id: release-metadata" in release
            and 'echo "release_created=true"' in release
            and 'echo "tag=$TAG"' in release
            and 'echo "version=$VERSION"' in release,
            "release metadata must emit explicit publication inputs")
    require(errors,
            'if: ${{ needs.release.outputs.release_created == \'true\' }}' in caller
            and 'release-publication-${{ needs.release.outputs.release_tag }}' in caller
            and "cancel-in-progress: false" in caller,
            "publication must be serialized and gated on normalized metadata")
    require(errors,
            "aarch64-apple-darwin" in build
            and "x86_64-apple-darwin" in build
            and "x86_64-unknown-linux-gnu" in build
            and "aarch64-unknown-linux-gnu" in build,
            "binary publication must build every supported release target")
    require(errors,
            "actions/attest-build-provenance@" in build
            and "attestations: write" in build
            and "id-token: write" in build
            and "subject-path: dist/*.tar.gz" in build,
            "every release archive must receive a GitHub build provenance attestation")
    require(errors,
            "id: publication-token" in publish
            and "permission-contents: write" in publish
            and "GH_TOKEN: ${{ steps.publication-token.outputs.token }}" in publish
            and 'release_artifacts.py upload --tag "$TAG" --expected-source "$EXPECTED_COMMIT"' in publish,
            "publication must use a fresh app token and source-bound overwrite helper")

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("release workflow builds and attests binaries before source-bound publication")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
