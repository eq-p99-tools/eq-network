#!/usr/bin/env python3
"""Fold changelog fragments from changelog.d/ into CHANGELOG.md.

Each pull request adds one fragment, changelog.d/<name>.md, holding the
bullet(s) it would have put under the Unreleased heading. Pull requests never
edit CHANGELOG.md, so two of them never conflict there.

This script moves every fragment into CHANGELOG.md, newest first (by the
commit that added it), right under the Unreleased heading, then deletes the
fragments with `git rm`. Run it from the repository root on an up-to-date
main branch, in a pull request of its own:

    python3 contrib/fold-changelog.py          # fold and stage
    python3 contrib/fold-changelog.py --check  # list fragments, change nothing
"""

import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
FRAGMENTS = ROOT / "changelog.d"
CHANGELOG = ROOT / "CHANGELOG.md"
HEADING = re.compile(r"^## \[?Unreleased\]?[ \t]*$", re.MULTILINE)


def added_at(path: Path) -> int:
    """Commit time of the commit that added the fragment.

    An uncommitted fragment sorts as the newest; folding refuses it anyway.
    """
    out = subprocess.run(
        ["git", "log", "--diff-filter=A", "--format=%ct", "-1", "--", str(path)],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout.strip()
    return int(out) if out else sys.maxsize


def fragments() -> list[Path]:
    found = [p for p in FRAGMENTS.glob("*.md") if p.name != "README.md"]
    # Newest first, matching the order entries were added to CHANGELOG.md.
    return sorted(found, key=lambda p: (added_at(p), p.name), reverse=True)


def main() -> int:
    found = fragments()
    if "--check" in sys.argv[1:]:
        for path in found:
            print(path.relative_to(ROOT))
        return 0
    if not found:
        print("No fragments to fold.")
        return 0
    untracked = [
        path.relative_to(ROOT)
        for path in found
        if subprocess.run(
            ["git", "ls-files", "--error-unmatch", "--", str(path)],
            cwd=ROOT, capture_output=True,
        ).returncode
        != 0
    ]
    if untracked:
        names = ", ".join(str(path) for path in untracked)
        sys.exit(f"Commit these fragments before folding (nothing was changed): {names}")
    text = CHANGELOG.read_text(encoding="utf-8")
    heading = HEADING.search(text)
    if heading is None:
        sys.exit("CHANGELOG.md has no Unreleased heading")
    entries = []
    for path in found:
        body = path.read_text(encoding="utf-8").strip("\n")
        if not body.strip():
            sys.exit(f"{path.relative_to(ROOT)} is empty")
        entries.append(body)
    insert_at = heading.end()
    rest = text[insert_at:].lstrip("\n")
    joined = "\n".join(entries) + "\n"
    if rest.startswith("#"):
        joined += "\n"  # keep a blank line before the next heading
    folded = text[:insert_at] + "\n\n" + joined + rest
    CHANGELOG.write_text(folded, encoding="utf-8")
    subprocess.run(
        ["git", "rm", "-q", "--", *[str(p) for p in found]], cwd=ROOT, check=True
    )
    subprocess.run(["git", "add", "--", str(CHANGELOG)], cwd=ROOT, check=True)
    print(f"Folded {len(found)} fragment(s) into CHANGELOG.md.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
