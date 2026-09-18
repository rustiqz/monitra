#!/usr/bin/env python3
"""Bump monitra's own [package] version, in Cargo.toml and Cargo.lock.

release-plz's own version cascade cannot make `monitra` move whenever an
internal crate does (release-plz.toml's dated NOTE explains why: `git_only`
always shells out to `cargo package`, which resolves named dependencies
against the real crates.io registry even for path overrides, and none of
these 13 crates are ever actually published). This patches the gap: the
release-plz.yml workflow calls this only when it has already decided some
other component is moving and monitra itself is not.

Usage: cascade-monitra-version.py <target-version>
"""

import re
import sys


def bump_cargo_toml(path: str, target: str) -> None:
    text = open(path).read()
    package = re.search(r"(?ms)^\[package\].*?(?=^\[)", text)
    if package is None:
        raise SystemExit(f"no [package] section in {path}")
    block = package.group(0)
    updated, replaced = re.subn(r'(?m)^version = ".*"$', f'version = "{target}"', block, count=1)
    if replaced == 0:
        raise SystemExit(f"no version key in the [package] section of {path}")
    open(path, "w").write(text.replace(block, updated, 1))


def bump_cargo_lock(path: str, target: str) -> None:
    text = open(path).read()
    pkg = re.search(r'(?ms)^\[\[package\]\]\nname = "monitra"\n.*?(?=^\[\[package\]\]|\Z)', text)
    if pkg is None:
        raise SystemExit(f'no [[package]] block named "monitra" in {path}')
    block = pkg.group(0)
    updated, replaced = re.subn(r'(?m)^version = ".*"$', f'version = "{target}"', block, count=1)
    if replaced == 0:
        raise SystemExit(f"no version key in monitra's [[package]] block in {path}")
    open(path, "w").write(text.replace(block, updated, 1))


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit("usage: cascade-monitra-version.py <target-version>")
    target = sys.argv[1]
    bump_cargo_toml("Cargo.toml", target)
    bump_cargo_lock("Cargo.lock", target)


if __name__ == "__main__":
    main()
