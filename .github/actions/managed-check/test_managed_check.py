"""Smoke-test the action's cache/build/quality-failure decisions."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).with_name("run.sh").resolve()
IMAGE = "sha256:" + "b" * 64
DOCKER = r'''
import json,os,shutil,sys
from pathlib import Path
args=sys.argv[1:]
with open('commands','a') as f: f.write(json.dumps(args)+'\n')
image='sha256:'+'b'*64
lock='sha256:'+'a'*64
if args[0]=='pull':
    sys.exit(1 if os.environ['MODE']=='miss' else 0)
elif args[:2]==['image','inspect']:
    if '--format' in args: print(image)
    else: print(json.dumps([{'Id':image,'Os':'linux','Architecture':'amd64','Config':{
        'Entrypoint':['/usr/local/bin/ayni-entrypoint'],'Labels':{
        'dev.ayni.environment.platform':'linux/amd64','dev.ayni.environment.lock-fingerprint':lock}}}]))
elif args[0]=='create': print('container')
elif args[0]=='cp':
    if os.environ['MODE']=='different-cli' and not Path('build-called').exists():
        Path(args[-1]).write_bytes(b'different CLI with the same version')
    else: shutil.copyfile(os.environ['AYNI_CI_CLI'],args[-1])
elif args[0]=='run' and args[-1]=='/bin/true':
    sys.exit(1 if os.environ['MODE']=='invalid-runtime' and not Path('build-called').exists() else 0)
elif args[0]=='run' and args[-1]!='/bin/true':
    if os.environ['MODE']!='missing-evidence':
        Path('.ayni/last').mkdir(parents=True,exist_ok=True)
        Path('.ayni/last/signals.json').write_text(json.dumps({'execution_mode':'managed',
            'environment_lock_fingerprint':lock,'completion':{'scope':'repository','state':'complete',
            'expected_targets':1,'detected_targets':1,'completed_targets':1,'skipped_targets':0,'issues':[]}}))
    print('# Quality evidence')
    sys.exit(1 if os.environ['MODE']=='quality-failure' else 0)
'''
CLI = r'''
from pathlib import Path
Path('build-called').write_text('yes')
Path('.ayni/environment').mkdir(parents=True,exist_ok=True)
Path('.ayni/environment/build.json').write_text('{"image_id":"sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"}')
'''


class LauncherTests(unittest.TestCase):
    def launch(self, mode):
        with tempfile.TemporaryDirectory(prefix="literal ; $path ") as directory:
            root = Path(directory).resolve()
            for name, body in (("docker", DOCKER), ("ayni", CLI)):
                path = root / name
                path.write_text(f"#!{sys.executable}\n" + body)
                path.chmod(0o755)
            (root / ".ayni.lock").write_text(json.dumps({"fingerprint": "sha256:" + "a" * 64,
                "repository": {"contract_path": ".ayni.toml"}, "targets": [{}]}))
            (root / ".ayni/last").mkdir(parents=True)
            (root / ".ayni/last/signals.json").write_text('{"stale":true}')
            env = dict(os.environ, PATH=f"{root}:{os.environ['PATH']}", MODE=mode,
                AYNI_CI_CLI=str(root / "ayni"), AYNI_CI_REPOSITORY="Org/Repo",
                AYNI_CI_PLATFORM="linux/amd64", AYNI_CI_REPORT=str(root / "report.md"),
                AYNI_CI_EXECUTOR="", GITHUB_OUTPUT=str(root / "outputs"))
            result = subprocess.run(["bash", str(SCRIPT)], cwd=root, env=env,
                                    capture_output=True, text=True)
            commands = [json.loads(line) for line in (root / "commands").read_text().splitlines()]
            return result.returncode, (root / "build-called").exists(), commands, (root / "report.md").read_text()

    def test_cache_hit_freezes_the_full_fingerprint_image(self):
        status, built, commands, _ = self.launch("hit")
        self.assertEqual(status, 0)
        self.assertFalse(built)
        self.assertEqual(commands[0][-1], "ghcr.io/org/repo:ayni-" + "a" * 64)
        self.assertTrue(all(IMAGE in args for args in commands if args[0] == "run"))

    def test_cache_miss_builds_then_checks(self):
        status, built, _, _ = self.launch("miss")
        self.assertEqual(status, 0)
        self.assertTrue(built)

    def test_quality_failure_keeps_report_and_never_rebuilds(self):
        status, built, _, report = self.launch("quality-failure")
        self.assertEqual(status, 1)
        self.assertFalse(built)
        self.assertIn("Quality evidence", report)

    def test_incompatible_cli_or_runtime_builds_before_quality(self):
        for mode in ("different-cli", "invalid-runtime"):
            with self.subTest(mode=mode):
                status, built, _, _ = self.launch(mode)
                self.assertEqual(status, 0)
                self.assertTrue(built)

    def test_missing_evidence_cannot_accept_a_previous_artifact(self):
        status, built, _, _ = self.launch("missing-evidence")
        self.assertNotEqual(status, 0)
        self.assertFalse(built)


if __name__ == "__main__":
    unittest.main()
