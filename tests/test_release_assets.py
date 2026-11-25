"""Offline release-repair verification; no GitHub API or network calls."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("verify_release_assets", Path(__file__).resolve().parents[1] / "tools" / "verify_release_assets.py")
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)


class ReleaseAssetsTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.version = "1.2.3"
        self.commit = "a" * 40
        self.evidence_name = f"ytTrayDL-v{self.version}-real-media-evidence.json"
        self.names = [f"ytTrayDL-Setup-v{self.version}-windows-x64.exe", f"ytTrayDL-v{self.version}-windows-x64.zip", self.evidence_name]
        (self.root / self.names[0]).write_bytes(b"setup")
        (self.root / self.names[1]).write_bytes(b"portable")
        self.evidence = {"schemaVersion": 1, "commit": self.commit, "status": "passed", "converterMediaTests": "passed", "scenarios": [{"name": name, "status": "passed"} for name in sorted(verifier.SCENARIOS)]}
        self.write_evidence()
        self.update_release()

    def write_evidence(self):
        (self.root / self.evidence_name).write_text(json.dumps(self.evidence), encoding="utf-8")

    def update_release(self):
        (self.root / "SHA256SUMS.txt").write_text("".join(f"{verifier.digest(self.root / name)}  {name}\n" for name in self.names), encoding="utf-8")
        self.release = {"draft": True, "tag_name": f"v{self.version}", "assets": [{"name": name, "state": "uploaded", "size": (self.root / name).stat().st_size, "digest": "sha256:" + verifier.digest(self.root / name)} for name in self.names + ["SHA256SUMS.txt"]]}

    def verify(self):
        verifier.verify(self.root, self.release, self.version, self.commit)

    def test_valid_draft(self):
        self.verify()

    def test_reject_missing_duplicate_or_tampered_asset(self):
        for change in ("missing", "duplicate", "tampered", "published"):
            with self.subTest(change=change):
                original = copy.deepcopy(self.release)
                if change == "missing": self.release["assets"].pop()
                if change == "duplicate": self.release["assets"][-1] = copy.deepcopy(self.release["assets"][0])
                if change == "tampered": self.release["assets"][0]["digest"] = "sha256:" + "0" * 64
                if change == "published": self.release["draft"] = False
                with self.assertRaises(ValueError): self.verify()
                self.release = original

    def test_reject_stale_or_incomplete_media_evidence_even_if_hashes_match(self):
        for key, value in (("commit", "b" * 40), ("converterMediaTests", "failed"), ("scenarios", [])):
            with self.subTest(key=key):
                original = copy.deepcopy(self.evidence)
                self.evidence[key] = value
                self.write_evidence()
                self.update_release()
                with self.assertRaises(ValueError): self.verify()
                self.evidence = original
                self.write_evidence()
                self.update_release()

    def test_duplicate_checksum_is_rejected_even_when_remote_digest_matches(self):
        checksum_file = self.root / "SHA256SUMS.txt"
        content = checksum_file.read_text(encoding="utf-8")
        checksum_file.write_text(content + content.splitlines()[0] + "\n", encoding="utf-8")
        asset = next(a for a in self.release["assets"] if a["name"] == "SHA256SUMS.txt")
        asset["size"] = checksum_file.stat().st_size
        asset["digest"] = "sha256:" + verifier.digest(checksum_file)
        with self.assertRaises(ValueError): self.verify()


if __name__ == "__main__":
    unittest.main()
