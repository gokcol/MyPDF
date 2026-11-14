#!/usr/bin/env python3
"""Generate GitHub and Microsoft Store notes from the bundled changelog."""
import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def render(changelog, version):
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Use a release version such as 2.4.3")
    notes = next((r for r in changelog["releases"] if r["version"] == version), None)
    if notes is None or not notes["changes"]:
        raise ValueError(f"No release notes for {version}; move completed unreleased changes into a release entry first")
    changes = notes["changes"]
    if not all(isinstance(c, str) and c.strip() for c in changes):
        raise ValueError("Release changes must be nonempty strings")
    github = f"## MyPDF {version}\n\n" + "\n".join(f"- {c}" for c in changes) + "\n"
    store = f"MyPDF {version}\n\n" + "\n".join(f"• {c}" for c in changes) + "\n"
    if len(store) > 1500:
        raise ValueError("Microsoft Store notes exceed 1,500 characters; shorten the shared release entry")
    return github, store


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--out-dir", type=Path, required=True)
    args = parser.parse_args()
    github, store = render(json.loads((ROOT / "changelog.json").read_text()), args.version)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    (args.out_dir / "github-release.md").write_text(github, encoding="utf-8")
    (args.out_dir / "release-notes.txt").write_text(store, encoding="utf-8")


if __name__ == "__main__":
    main()
