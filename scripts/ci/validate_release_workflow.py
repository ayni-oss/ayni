"""Validate source-bound release and tagged documentation workflows."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
DOCS_WORKFLOW = ROOT / ".github" / "workflows" / "docs.yml"
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
    docs = DOCS_WORKFLOW.read_text()
    publication = (WORKFLOW.parent / "release-publication.yml").read_text()
    errors: list[str] = []
    require(errors,
            sorted(path.name for path in (ROOT / ".github/docker").glob("*.Dockerfile"))
            == ["ayni-builder.Dockerfile"],
            "Ayni must maintain one publishable image recipe: ayni-builder")

    try:
        release = job_block(source, "release")
        sync_lock = job_block(source, "sync-release-lock")
        caller = job_block(source, "publication")
        completion = job_block(source, "release-completion")
        build = job_block(publication, "build")
        publish = job_block(publication, "publish")
        executor = job_block(publication, "executor")
        executor_manifest = job_block(publication, "executor-manifest")
    except ValueError as error:
        print(error, file=sys.stderr)
        return 1

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
            "release_pr_available: ${{ steps.release-pr.outputs.available }}" in release
            and "release_pr_branch: ${{ steps.release-pr.outputs.branch }}" in release
            and "id: release-pr" in release
            and 'echo "branch=$branch"' in release,
            "release metadata must expose an available release pull-request branch")
    require(errors,
            "needs: release" in sync_lock
            and "needs.release.outputs.release_pr_available == 'true'" in sync_lock
            and "release-pr-maintenance" in sync_lock
            and "jdx/mise-action@" in sync_lock
            and "version: 2026.6.14" in sync_lock
            and "cargo run --locked -p ayni-cli -- env lock --repo-root ." in sync_lock
            and 'cmp "$RUNNER_TEMP/release-lock.first" .ayni.lock' in sync_lock
            and 'git commit -m "chore(release): refresh Ayni lock"' in sync_lock,
            "an available release pull request must receive one deterministic Ayni-lock refresh")
    require(errors,
            'if: ${{ needs.release.outputs.release_created == \'true\' }}' in caller
            and 'release-publication-${{ needs.release.outputs.release_tag }}' in caller
            and "cancel-in-progress: false" in caller,
            "publication must be serialized and gated on normalized metadata")
    require(errors,
            "packages: write" in caller
            and "needs: [release, publication]" in completion
            and "if: ${{ always() }}" in completion
            and 'test "$PUBLICATION_RESULT" = "success"' in completion
            and "release_artifacts.py verify --tag" in completion
            and 'docker buildx imagetools inspect "$EXECUTOR_IMAGE"' in completion
            and 'docker pull --platform linux/amd64 "$EXECUTOR_IMAGE"' in completion,
            "release completion must fail closed unless public assets and executor validate")
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
    require(errors,
            "packages: write" in executor
            and "ubuntu-24.04-arm" in executor
            and "x86_64-unknown-linux-gnu" in executor
            and "aarch64-unknown-linux-gnu" in executor
            and "ayni-builder:${{ inputs.version }}-${{ matrix.suffix }}" in executor
            and "ayni-builder.Dockerfile" in executor
            and '"$IMAGE" docker --version' in executor
            and '"$IMAGE" docker buildx version' in executor
            and 'docker push "$IMAGE"' in executor
            and "docker buildx imagetools create" in executor_manifest
            and "ayni-builder:${{ inputs.version }}" in executor_manifest
            and "linux/amd64" in executor_manifest
            and "linux/arm64" in executor_manifest
            and 'docker logout ghcr.io || true' in executor_manifest
            and 'echo "executor_image=$IMAGE@$digest"' in executor_manifest,
            "release publication must publish and validate the versioned multi-architecture builder")
    require(errors,
            "tags:\n      - 'ayni-v*'" in docs
            and "branches:" not in docs
            and "cargo doc-cli > docs/cli.md" in docs
            and "npm ci" in docs
            and "npm run docs:build" in docs
            and "VITEPRESS_BASE: /" in docs
            and "actions/upload-pages-artifact@" in docs
            and "actions/deploy-pages@" in docs
            and "pages: write" in docs
            and "id-token: write" in docs,
            "documentation must build and deploy only tagged release source")

    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print("release workflow publishes source-bound binaries and one versioned multi-architecture builder")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
