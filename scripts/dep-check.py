#!/usr/bin/env python3
"""Enforce the DESIGN.md 3.2 dependency DAG (P4 / ADR-005 / ADR-007).

A boundary that is only a convention gets violated. This makes it a build error.
Exits 0 if the graph is clean, 1 otherwise.
"""
import sys, pathlib, tomllib

# Mirror of the DESIGN.md 3.2 table. Update BOTH together, never just this one.
ALLOWED = {
    "monitra-models":       set(),
    "monitra-provider":     {"monitra-models"},
    "monitra-storage":      {"monitra-models", "monitra-provider"},
    "store-postgres":       {"monitra-models", "monitra-provider"},
    "cache-redis":          {"monitra-models", "monitra-provider"},
    "notify-webhook":       {"monitra-models", "monitra-provider"},
    "notify-slack":         {"monitra-models", "monitra-provider"},
    "collector-kubernetes": {"monitra-models", "monitra-provider"},  # ADR-008
    "monitra-probe":        {"monitra-models"},               # ADR-011 — shared by engine and agent
    "monitra-engine":       {"monitra-models", "monitra-provider", "monitra-probe"},
    "monitra-backend":      {"monitra-models", "monitra-provider", "monitra-engine"},
    "monitra-tui":          {"monitra-models"},               # ADR-009 — no provider/storage, ever
    "monitra-agent":        {"monitra-models", "monitra-probe"},  # ADR-008/ADR-011
    "monitra-cli":          {"monitra-models"},
}
DEP_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")

def internal_deps(manifest, known):
    """Internal deps = path deps, or deps naming a known workspace crate."""
    found = set()
    for section in DEP_SECTIONS:
        for name, spec in (manifest.get(section) or {}).items():
            if isinstance(spec, dict) and ("path" in spec or "workspace" in spec):
                if name in known:
                    found.add(name)
            elif name in known:
                found.add(name)
    for target in (manifest.get("target") or {}).values():
        for section in DEP_SECTIONS:
            for name in (target.get(section) or {}):
                if name in known:
                    found.add(name)
    return found

def main():
    root = pathlib.Path(__file__).resolve().parent.parent
    crates_dir = root / "crates"
    if not crates_dir.is_dir():
        print("dep-check: no crates/ directory yet — nothing to check")
        return 0

    manifests = sorted(crates_dir.glob("*/Cargo.toml"))
    if not manifests:
        print("dep-check: no crate manifests yet — nothing to check")
        return 0

    known = set(ALLOWED)
    violations, unpoliced = [], []

    for path in manifests:
        with path.open("rb") as fh:
            manifest = tomllib.load(fh)
        name = (manifest.get("package") or {}).get("name", path.parent.name)
        if name not in ALLOWED:
            unpoliced.append(name)
            continue
        for dep in sorted(internal_deps(manifest, known) - {name}):
            if dep not in ALLOWED[name]:
                violations.append((name, dep))

    for name in unpoliced:
        print(f"FAIL  crate '{name}' has no entry in the dependency policy.")
        print( "      Add it to DESIGN.md 3.2 AND scripts/dep-check.py before use.")
    for name, dep in violations:
        allowed = ", ".join(sorted(ALLOWED[name])) or "(nothing internal)"
        print(f"FAIL  {name} -> {dep} is forbidden. {name} may depend on: {allowed}")

    if violations or unpoliced:
        print(f"\ndep-check: FAILED ({len(violations) + len(unpoliced)} problem(s)) — see DESIGN.md 3.2")
        return 1

    print(f"dep-check: OK — {len(manifests)} crate(s), dependency DAG matches DESIGN.md 3.2")
    return 0

if __name__ == "__main__":
    sys.exit(main())
