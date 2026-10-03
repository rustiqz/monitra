#!/usr/bin/env python3
"""Compute and apply the next Monitra release from merged PR titles.

Replaces release-plz (see CONTRIBUTING.md "Releases"). release-plz is built
around a per-crate release PR; Monitra ships one binary, so the version lives
in `[workspace.package]` only (every crate inherits it) and is bumped straight from what landed on `main`.

Source of truth for "what changed" is the first-parent history since the last
release tag. A merge commit contributes its PR title (GitHub puts it on the
first body line; `pr-title-lint` guarantees it is a Conventional Commit); any
other commit (squash merge, direct push) contributes its own subject. Commits
inside a PR branch are ignored — they are not required to be conventional.

Bump rules (pre-1.0, so a breaking change moves the minor, not the major):
  breaking (`!` or `BREAKING CHANGE`)  -> major  (minor while major == 0)
  feat                                 -> minor
  fix / perf / refactor                -> patch
  docs / test / build / ci / chore     -> no release on their own

Usage:
  release.py plan   [--notes FILE]   print the decision; write notes; set GITHUB_OUTPUT
  release.py apply  <version> [--notes FILE]   write the workspace version and CHANGELOG.md (Cargo.lock is
                                refreshed by the caller with `cargo update --workspace`)
"""

import argparse
import datetime
import os
import re
import subprocess
import sys

REPO_URL = "https://github.com/rustiqz/monitra"
# `monitra-v*` is the pre-automation tag scheme; honoured only as a baseline
# so the first automated release does not re-announce the whole history.
TAG_PATTERNS = ("v[0-9]*", "monitra-v[0-9]*")
RELEASE_COMMIT_PREFIX = "chore(release)"

CONVENTIONAL = re.compile(r"^(?P<type>[a-z]+)(?:\((?P<scope>[^)]*)\))?(?P<bang>!)?:\s*(?P<desc>.+)$")
PR_NUMBER = re.compile(r"^Merge pull request #(?P<n>\d+) ")

# Changelog section for each type that appears in release notes. Types absent
# here (docs, test, build, ci, chore) never reach the notes.
SECTIONS = (
    ("breaking", "Breaking changes"),
    ("feat", "Added"),
    ("fix", "Fixed"),
    ("perf", "Changed"),
    ("refactor", "Changed"),
)


def git(*args):
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout


def parse_version(text):
    m = re.fullmatch(r"(\d+)\.(\d+)\.(\d+)", text)
    if m is None:
        raise ValueError(f"not a plain MAJOR.MINOR.PATCH version: {text!r}")
    return tuple(int(p) for p in m.groups())


def last_release():
    """Highest release tag as (version tuple, tag name), or None if never released."""
    best = None
    for pattern in TAG_PATTERNS:
        for tag in git("tag", "--list", pattern).split():
            try:
                version = parse_version(tag.split("v", 1)[1])
            except ValueError:
                continue
            if best is None or version > best[0]:
                best = (version, tag)
    return best


def read_commits(since_tag):
    """First-parent commits since `since_tag` as (subject, body, parent_count, sha)."""
    rng = f"{since_tag}..HEAD" if since_tag else "HEAD"
    raw = git("log", "--first-parent", "--format=%H%x1f%P%x1f%s%x1f%b%x1e", rng)
    commits = []
    for record in raw.split("\x1e"):
        record = record.strip("\n")
        if not record:
            continue
        sha, parents, subject, body = record.split("\x1f", 3)
        commits.append((subject, body, len(parents.split()), sha))
    return commits


def title_of(subject, body, parent_count):
    """The line that carries the change's conventional-commit title."""
    if parent_count > 1:
        for line in body.splitlines():
            if line.strip():
                return line.strip()
        return None
    return subject


def parse_changes(commits):
    """Turn commits into [{type, scope, desc, breaking, pr, sha}], skipping non-conventional ones."""
    changes, ignored = [], []
    for subject, body, parent_count, sha in commits:
        if subject.startswith(RELEASE_COMMIT_PREFIX):
            continue
        title = title_of(subject, body, parent_count)
        m = CONVENTIONAL.match(title) if title else None
        if m is None:
            ignored.append((sha[:7], title or subject))
            continue
        pr = PR_NUMBER.match(subject)
        changes.append(
            {
                "type": m["type"],
                "scope": m["scope"],
                "desc": m["desc"].strip(),
                "breaking": bool(m["bang"]) or "BREAKING CHANGE" in body,
                "pr": pr["n"] if pr else None,
                "sha": sha,
            }
        )
    return changes, ignored


def decide_bump(changes, current):
    """'major' | 'minor' | 'patch' | None."""
    if any(c["breaking"] for c in changes):
        return "minor" if current[0] == 0 else "major"
    if any(c["type"] == "feat" for c in changes):
        return "minor"
    if any(c["type"] in ("fix", "perf", "refactor") for c in changes):
        return "patch"
    return None


def next_version(current, bump):
    major, minor, patch = current
    if bump == "major":
        return (major + 1, 0, 0)
    if bump == "minor":
        return (major, minor + 1, 0)
    return (major, minor, patch + 1)


def render_notes(version, previous_tag, changes, today):
    grouped = {}
    for c in changes:
        key = "breaking" if c["breaking"] else c["type"]
        heading = dict(SECTIONS).get(key)
        if heading:
            grouped.setdefault(heading, []).append(c)

    lines = [f"## [{version}]({REPO_URL}/releases/tag/v{version}) - {today}", ""]
    for _, heading in SECTIONS:
        entries = grouped.pop(heading, None)
        if not entries:
            continue
        lines += [f"### {heading}", ""]
        for c in entries:
            scope = f"*({c['scope']})* " if c["scope"] else ""
            ref = f" ([#{c['pr']}]({REPO_URL}/pull/{c['pr']}))" if c["pr"] else ""
            lines.append(f"- {scope}{c['desc']}{ref}")
        lines.append("")
    if previous_tag:
        lines += [f"**Full changelog**: {REPO_URL}/compare/{previous_tag}...v{version}", ""]
    return "\n".join(lines)


def read_package_version(cargo_toml="Cargo.toml"):
    text = open(cargo_toml).read()
    section = re.search(r"(?ms)^\[workspace\.package\].*?(?=^\[|\Z)", text)
    m = re.search(r'(?m)^version = "(.*)"$', section.group(0)) if section else None
    if m is None:
        raise SystemExit(f"release: no version in the [workspace.package] section of {cargo_toml}")
    return m.group(1)


def write_cargo_toml(path, target):
    text = open(path).read()
    section = re.search(r"(?ms)^\[workspace\.package\].*?(?=^\[|\Z)", text)
    if section is None:
        raise SystemExit(f"release: no [workspace.package] section in {path}")
    block = section.group(0)
    updated, n = re.subn(r'(?m)^version = ".*"$', f'version = "{target}"', block, count=1)
    if n == 0:
        raise SystemExit(f"release: no version key in the [workspace.package] section of {path}")
    open(path, "w").write(text.replace(block, updated, 1))


def prepend_changelog(path, notes):
    marker = "## [Unreleased]\n"
    text = open(path).read()
    if marker not in text:
        raise SystemExit(f"release: no '{marker.strip()}' heading in {path}")
    head, tail = text.split(marker, 1)
    open(path, "w").write(f"{head}{marker}\n{notes.rstrip()}\n{tail}")


def set_output(**values):
    path = os.environ.get("GITHUB_OUTPUT")
    if not path:
        return
    with open(path, "a") as out:
        for key, value in values.items():
            out.write(f"{key}={value}\n")


def cmd_plan(args):
    last = last_release()
    current = last[0] if last else parse_version(read_package_version())
    previous_tag = last[1] if last else None
    changes, ignored = parse_changes(read_commits(previous_tag))

    for sha, title in ignored:
        print(f"release: ignoring {sha} — not a Conventional Commit title: {title!r}", file=sys.stderr)

    bump = decide_bump(changes, current)
    if bump is None:
        print(f"release: no releasable changes since {previous_tag or 'the start of history'} — nothing to do")
        set_output(released="false")
        return 0

    version = ".".join(str(p) for p in next_version(current, bump))
    notes = render_notes(version, previous_tag, changes, datetime.date.today().isoformat())
    with open(args.notes, "w") as f:
        f.write(notes)
    print(f"release: {'.'.join(map(str, current))} -> {version} ({bump})")
    print(notes)
    set_output(released="true", version=version, bump=bump)
    return 0


def cmd_apply(args):
    parse_version(args.version)
    write_cargo_toml("Cargo.toml", args.version)
    prepend_changelog("CHANGELOG.md", open(args.notes).read())
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    plan = sub.add_parser("plan")
    plan.add_argument("--notes", default="release-notes.md")
    plan.set_defaults(func=cmd_plan)
    apply = sub.add_parser("apply")
    apply.add_argument("version")
    apply.add_argument("--notes", default="release-notes.md")
    apply.set_defaults(func=cmd_apply)
    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
