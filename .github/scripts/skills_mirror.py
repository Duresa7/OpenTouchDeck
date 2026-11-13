#!/usr/bin/env python3
"""Keep .claude/skills a copy of .agents/skills, the canonical agent skills.

    python .github/scripts/skills_mirror.py check
    python .github/scripts/skills_mirror.py sync

Codex and other agents read .agents/skills; Claude Code reads only .claude/skills. Each skill folder in
.agents/skills is copied byte for byte to .claude/skills. Loose files at the top of .agents/skills, such as
README.md, are not copied. Copies rather than symlinks, so Windows checkouts work.

check exits 1 and lists every difference when the copy has drifted. sync rewrites .claude/skills to match.
Edit skills in .agents/skills, then run sync.
"""

import argparse
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parents[2]
CANONICAL = ROOT / ".agents" / "skills"
MIRROR = ROOT / ".claude" / "skills"
# Never mirror local caches; .gitignore keeps them out of git as well.
SKIPPED = {"__pycache__", ".DS_Store", "Thumbs.db", "desktop.ini"}


def files(top):
    if not top.is_dir():
        return {}
    found = {}
    for skill in sorted(p for p in top.iterdir() if p.is_dir() and p.name not in SKIPPED):
        for path in sorted(skill.rglob("*")):
            relative = path.relative_to(top)
            if path.is_file() and not SKIPPED.intersection(relative.parts):
                found[relative.as_posix()] = path
    return found


def differences():
    want, have = files(CANONICAL), files(MIRROR)
    problems = [f"missing from .claude/skills: {name}" for name in sorted(want.keys() - have.keys())]
    problems += [f"not in .agents/skills: {name}" for name in sorted(have.keys() - want.keys())]
    problems += [f"differs: {name}" for name in sorted(want.keys() & have.keys())
                 if want[name].read_bytes() != have[name].read_bytes()]
    return problems


def check():
    problems = differences()
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        print("\n.claude/skills must copy .agents/skills. Edit .agents/skills, then run:\n"
              "    python .github/scripts/skills_mirror.py sync", file=sys.stderr)
        return 1
    print(f"ok: .claude/skills matches .agents/skills ({len(files(CANONICAL))} files)")
    return 0


def sync():
    if not CANONICAL.is_dir():
        print(f"error: {CANONICAL.relative_to(ROOT).as_posix()} not found", file=sys.stderr)
        return 1
    MIRROR.mkdir(parents=True, exist_ok=True)
    wanted = {p.name for p in CANONICAL.iterdir() if p.is_dir() and p.name not in SKIPPED}
    for entry in MIRROR.iterdir():
        if entry.name not in wanted:
            # A junction or symlink is removed as a link; its target is left alone.
            if entry.is_symlink() or entry.is_junction() or entry.is_file():
                entry.unlink()
            else:
                shutil.rmtree(entry)
    for name in sorted(wanted):
        target = MIRROR / name
        if target.is_symlink() or target.is_junction():
            target.unlink()
        elif target.exists():
            shutil.rmtree(target)
        shutil.copytree(CANONICAL / name, target, ignore=shutil.ignore_patterns(*SKIPPED))
    return check()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("command", choices=("check", "sync"))
    args = parser.parse_args()
    return check() if args.command == "check" else sync()


if __name__ == "__main__":
    sys.exit(main())
