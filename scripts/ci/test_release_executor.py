"""Execute release workflow JSON checks rather than only inspecting their text."""

import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest


WORKFLOW = Path(__file__).resolve().parents[2] / ".github/workflows/release-publication.yml"


def checks(marker):
    return [line.strip() for line in WORKFLOW.read_text().splitlines()
            if marker in line and line.lstrip().startswith("test ")]


def execute(lines, **variables):
    return subprocess.run(
        ["bash", "-c", "set -euo pipefail\n" + "\n".join(lines)],
        env={**os.environ, **variables}, capture_output=True, text=True,
    )


class ReleaseExecutorTests(unittest.TestCase):
    def test_recovery_uses_pinned_workflow_recipe_only_when_tag_has_no_builder(self):
        source = WORKFLOW.read_text()
        selection = source[source.index("          recipe_root=."):source.index('          archive="release/')]
        selection = "\n".join(line[10:] for line in selection.splitlines())
        for tagged_builder in (True, False):
            with tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                recovery = root / "factory-source/.github/docker"
                recovery.mkdir(parents=True)
                (recovery / "ayni-builder.versions").write_text("DEBIAN_IMAGE=workflow-debian\n")
                if tagged_builder:
                    tagged = root / ".github/docker"
                    tagged.mkdir(parents=True)
                    (tagged / "ayni-builder.Dockerfile").write_text("FROM debian\n")
                    (tagged / "ayni-builder.versions").write_text("DEBIAN_IMAGE=tagged-debian\n")
                result = subprocess.run(
                    ["bash", "-ec", selection + '\nprintf "%s %s" "$recipe_revision" "$DEBIAN_IMAGE"'],
                    cwd=root, env={**os.environ, "EXPECTED_COMMIT": "tagged-source", "WORKFLOW_COMMIT": "workflow-source"},
                    capture_output=True, text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, "tagged-source tagged-debian" if tagged_builder
                                 else "workflow-source workflow-debian")

    def test_both_label_checks_accept_valid_metadata_and_reject_each_mismatch(self):
        lines = checks('<<<"$labels"')
        self.assertEqual(len(lines), 8)
        labels = {
            "org.opencontainers.image.revision": "a" * 40,
            "org.opencontainers.image.version": "0.14.1",
            "dev.ayni.executor.lock-schema": "0.9.0",
            "dev.ayni.executor.recipe": "2",
        }
        variables = dict(EXPECTED_COMMIT="a" * 40, EXPECTED_VERSION="0.14.1")
        for block in (lines[:4], lines[4:]):
            result = execute(block, labels=json.dumps(labels), **variables)
            self.assertEqual(result.returncode, 0, result.stderr)
            for key in labels:
                for value in (None, "wrong"):
                    invalid = {**labels, key: value}
                    self.assertNotEqual(execute(block, labels=json.dumps(invalid),
                                                **variables).returncode, 0)

    def test_manifest_requires_exactly_both_linux_architectures(self):
        lines = checks('<<<"$manifest"')
        self.assertEqual(len(lines), 1)
        for platforms, valid in (
            (["linux/amd64", "linux/arm64"], True),
            (["linux/arm64", "linux/amd64"], True),
            (["linux/amd64"], False),
            (["linux/arm64"], False),
            (["linux/amd64", "linux/arm64", "linux/amd64"], False),
            (["linux/amd64", "linux/arm64", "unknown/unknown"], False),
        ):
            manifest = {"manifests": [
                {"platform": dict(zip(("os", "architecture"), platform.split("/")))}
                for platform in platforms
            ]}
            result = execute(lines, manifest=json.dumps(manifest))
            self.assertEqual(result.returncode == 0, valid, result.stderr)

    def test_executor_digest_requires_sha256_identity(self):
        lines = [line.strip() for line in WORKFLOW.read_text().splitlines()
                 if line.strip().startswith('[[ "$digest"')]
        self.assertEqual(len(lines), 1)
        for digest, valid in (("sha256:" + "a" * 64, True), ("", False),
                              ("<no value>", False), ("sha256:abc", False)):
            self.assertEqual(execute(lines, digest=digest).returncode == 0, valid)


if __name__ == "__main__":
    unittest.main()
