"""Exercise the repair verifier on the just-built real package artifacts.

Remote release metadata is simulated from local hashes. This checks package and
media-evidence shape, not GitHub upload or actual draft publication.
"""
import os
from pathlib import Path

from verify_release_assets import digest, verify


def main():
    version = os.environ["APP_VERSION"]
    commit = os.environ["GITHUB_SHA"]
    root = Path("release")
    assets = [
        {"name": path.name, "state": "uploaded", "size": path.stat().st_size, "digest": "sha256:" + digest(path)}
        for path in root.iterdir()
        if path.is_file() and not path.is_symlink()
    ]
    verify(root, {"draft": True, "tag_name": f"v{version}", "assets": assets}, version, commit)
    print("Repair verifier accepted real build artifacts with simulated release metadata")


if __name__ == "__main__":
    main()
