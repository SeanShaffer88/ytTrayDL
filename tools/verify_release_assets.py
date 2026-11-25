"""Offline, fail-closed verification of checked artifacts against a draft release.

This verifier never downloads, uploads or publishes anything. The caller must
first authenticate the build run and tag, and only publish after this succeeds.
"""
import hashlib
import json
import re
import sys
from pathlib import Path


SCENARIOS = {"video", "merge", "audio", "subtitles", "playlist", "retry", "pause", "cancel", "shutdown"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    value = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            value.update(chunk)
    return value.hexdigest()


def verify(root, release, version, commit):
    require(re.fullmatch(r"\d+\.\d+\.\d+", version) is not None, "Invalid version")
    require(re.fullmatch(r"[0-9a-f]{40}", commit) is not None, "Invalid commit")
    setup = f"ytTrayDL-Setup-v{version}-windows-x64.exe"
    portable = f"ytTrayDL-v{version}-windows-x64.zip"
    evidence_name = f"ytTrayDL-v{version}-real-media-evidence.json"
    expected = {"SHA256SUMS.txt", setup, portable, evidence_name}
    require({path.name for path in root.iterdir()} == expected, "Unexpected build artifact inventory")
    require(all((root / name).is_file() and not (root / name).is_symlink() for name in expected), "Invalid artifact file")
    require(release.get("draft") is True and release.get("tag_name") == f"v{version}", "Expected matching draft")
    assets = release.get("assets", [])
    require(len(assets) == len(expected), "Unexpected release asset count")
    remote = {asset["name"]: asset for asset in assets}
    require(set(remote) == expected, "Unexpected or duplicate release asset inventory")
    for name in expected:
        path = root / name
        asset = remote[name]
        require(asset.get("state") == "uploaded" and asset.get("size") == path.stat().st_size, f"Invalid uploaded asset: {name}")
        require(asset.get("digest") == "sha256:" + digest(path), f"Uploaded digest mismatch: {name}")
    sums = {}
    for line in (root / "SHA256SUMS.txt").read_text(encoding="utf-8").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  ([^/\\]+)", line)
        require(match is not None, "Malformed checksum line")
        checksum, name = match.groups()
        require(name not in sums, "Duplicate checksum entry")
        sums[name] = checksum
    require(set(sums) == expected - {"SHA256SUMS.txt"}, "Unexpected checksum inventory")
    for name, checksum in sums.items():
        require(digest(root / name) == checksum, f"Checksum mismatch: {name}")
    evidence = json.loads((root / evidence_name).read_text(encoding="utf-8"))
    require(evidence.get("schemaVersion") == 1 and evidence.get("commit") == commit, "Media evidence commit mismatch")
    require(evidence.get("status") == "passed" and evidence.get("converterMediaTests") == "passed", "Media evidence did not pass")
    scenarios = evidence.get("scenarios", [])
    require(len(scenarios) == len(SCENARIOS) and {s.get("name") for s in scenarios} == SCENARIOS, "Unexpected media scenarios")
    require(all(s.get("status") == "passed" for s in scenarios), "Media scenario failed")


def main():
    if len(sys.argv) != 5:
        raise SystemExit("Expected checked-directory release-json version commit")
    root, release_path, version, commit = sys.argv[1:]
    try:
        verify(Path(root), json.loads(Path(release_path).read_text(encoding="utf-8")), version, commit)
    except (ValueError, OSError, KeyError, TypeError, json.JSONDecodeError) as error:
        raise SystemExit(f"Release asset verification failed: {error}") from error
    print("Draft assets, checksums and media evidence verified")


if __name__ == "__main__":
    main()
