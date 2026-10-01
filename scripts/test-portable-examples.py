#!/usr/bin/env python3
"""Exercise existing five-language fixtures in a clean certified-image consumer."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).parent / "ci"))
import composition

TEST_PUBLIC_KEY = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"


def run(cli, executor):
    repository = Path(__file__).resolve().parent.parent
    image = None
    with tempfile.TemporaryDirectory(prefix="ayni-portable-examples-") as temporary:
        root = Path(temporary)
        builder = root / "builder"
        targets = composition.materialize(repository, builder, "all-five")
        config = builder / ".ayni.toml"
        # Portability exercises native test execution and prepared dependencies;
        # coverage thresholds are verified by the existing quality fixture suite.
        policy = config.read_text()
        for signal in ("coverage", "size", "complexity", "deps"):
            policy = policy.replace(f"{signal} = true", f"{signal} = false", 1)
        config.write_text(policy)
        with config.open("a") as output:
            output.write(f'\n[environment.certificate.trusted_keys]\nissue-35-test = "{TEST_PUBLIC_KEY}"\n')
        environment = os.environ.copy()
        # Public RFC 8032 test vector; never used for published environments.
        environment["AYNI_ENV_CERTIFICATE_KEY_ID"] = "issue-35-test"
        environment["AYNI_ENV_CERTIFICATE_SIGNING_KEY"] = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60"
        base = json.loads((repository / ".ayni.lock").read_text())["provisioning_base"]
        if sys.platform == "linux":
            # Exercise the resolver shipped in the locked base, independent of the runner host.
            tools = root / "tools"
            tools.mkdir()
            container = subprocess.check_output(["docker", "create", base["reference"] + "@" + base["digest"]], text=True).strip()
            try:
                subprocess.run(["docker", "cp", container + ":/usr/local/bin/mise", str(tools / "mise")], check=True)
            finally:
                subprocess.run(["docker", "rm", container], check=True, stdout=subprocess.DEVNULL)
            environment["PATH"] = str(tools) + os.pathsep + environment["PATH"]
        elif not shutil.which("mise"):
            raise RuntimeError("mise is required on non-Linux contributor hosts")
        try:
            subprocess.run([cli, "env", "lock", "--repo-root", str(builder), "--base", base["reference"] + "@" + base["digest"]], check=True, env=environment)
            subprocess.run(["git", "-C", str(builder), "add", "."], check=True)
            subprocess.run([cli, "env", "build", "--repo-root", str(builder), "--executor-image", executor], check=True, env=environment)
            image = json.loads((builder / ".ayni/environment/build.json").read_text())["image_id"]
            baseline = subprocess.run([cli, "check", "--config", str(config), "--output", "json"], text=True, capture_output=True)
            if baseline.returncode != 0:
                raise RuntimeError(f"ordinary managed examples failed: {baseline.stderr}\n{baseline.stdout}")
            consumer = root / "consumer"
            shutil.copytree(builder, consumer, ignore=shutil.ignore_patterns(".ayni", ".git"))
            output = consumer / ".ayni"
            output.mkdir(mode=0o777)
            output.chmod(0o777)
            result = subprocess.run([
                "docker", "run", "--rm", "--read-only", "--network", "none",
                "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--user", "10001:10001",
                "--mount", f"type=bind,source={consumer},target=/source,readonly",
                "--mount", f"type=bind,source={output},target=/source/.ayni",
                "--tmpfs", "/workspace:rw,exec,nosuid,size=4g,mode=1777",
                "--tmpfs", "/tmp:rw,exec,nosuid,size=8g,mode=1777",
                "--workdir", "/source", "--entrypoint", "/bin/sh", image,
                # These disposable outputs must be removable by the host's different UID on Linux.
                "-c", "umask 000; exec /usr/local/bin/ayni check --output json",
            ], text=True, capture_output=True)
            print(result.stderr, file=sys.stderr)
            if result.returncode not in (0, 1):
                raise RuntimeError(f"portable examples exited {result.returncode}: {result.stdout}")
            document = json.loads(result.stdout)
            expected = {(language, name) for name, language in targets.items()}
            actual = {(row["language"], row["scope"]["path"]) for row in document["rows"]}
            assert actual == expected and len(document["rows"]) == len(expected)
            assert result.returncode == 0
            assert all(row["kind"] == "test" and row["pass"] for row in document["rows"])
            assert document["completion"]["completed_targets"] == len(expected)
            reference = json.loads(baseline.stdout)
            def test_counts(artifact):
                return sorted((row["language"], row["result"]["total_tests"], row["result"]["passed"], row["result"]["failed"]) for row in artifact["rows"])
            assert all(row["result"]["total_tests"] > 0 for row in document["rows"])
            assert test_counts(document) == test_counts(reference)
            evidence = json.loads((output / "last/execution.json").read_text())
            digest = "sha256:" + hashlib.sha256((output / "last/signals.json").read_bytes()).hexdigest()
            assert evidence["artifact_digest"] == digest
            assert evidence["prebuilt_runtime"]["certificate"] == document["prebuilt_runtime"]["certificate"]
            assert not (output / "environment/build.json").exists()
            print("Verified portable prepared dependencies and quality evidence for all five languages.")
        finally:
            if image:
                subprocess.run(["docker", "image", "rm", image], check=False, stdout=subprocess.DEVNULL)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", required=True)
    parser.add_argument("--executor", required=True)
    arguments = parser.parse_args()
    run(str(Path(arguments.cli).resolve()), arguments.executor)
