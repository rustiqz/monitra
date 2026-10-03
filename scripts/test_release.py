#!/usr/bin/env python3
"""Tests for scripts/release.py — run with `python3 -m unittest scripts/test_release.py`.

The integration cases build a throwaway git repo with real merge commits, because
the one thing that matters here (reading a PR title out of a merge commit body)
is exactly what a pure-function test would have to fake.
"""

import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(__file__))
import release  # noqa: E402


def change(type_, breaking=False, scope=None):
    return {"type": type_, "scope": scope, "desc": "x", "breaking": breaking, "pr": None, "sha": "0"}


class Bump(unittest.TestCase):
    def test_rules(self):
        self.assertEqual(release.decide_bump([change("fix")], (1, 2, 3)), "patch")
        self.assertEqual(release.decide_bump([change("perf")], (1, 2, 3)), "patch")
        self.assertEqual(release.decide_bump([change("fix"), change("feat")], (1, 2, 3)), "minor")
        self.assertEqual(release.decide_bump([change("fix", breaking=True)], (1, 2, 3)), "major")

    def test_breaking_is_minor_before_1_0(self):
        self.assertEqual(release.decide_bump([change("feat", breaking=True)], (0, 3, 1)), "minor")

    def test_housekeeping_alone_does_not_release(self):
        kinds = ["docs", "test", "build", "ci", "chore"]
        self.assertIsNone(release.decide_bump([change(k) for k in kinds], (1, 0, 0)))

    def test_next_version(self):
        self.assertEqual(release.next_version((0, 1, 4), "patch"), (0, 1, 5))
        self.assertEqual(release.next_version((0, 1, 4), "minor"), (0, 2, 0))
        self.assertEqual(release.next_version((1, 1, 4), "major"), (2, 0, 0))

    def test_deliberate_one_zero_override(self):
        self.assertEqual(release.release_version([dict(change("chore"), release_as="1.0.0")], (0, 9, 4)), (1, 0, 0))
        self.assertEqual(release.release_version([change("feat", breaking=True)], (0, 9, 4)), (0, 10, 0))

    def test_override_rejects_other_versions_and_repeat_use(self):
        for current, value in [((0, 9, 4), "1.1.0"), ((1, 0, 0), "1.0.0")]:
            with self.subTest(current=current, value=value), self.assertRaises(SystemExit):
                release.release_version([dict(change("chore"), release_as=value)], current)
        with self.assertRaises(SystemExit):
            release.release_version([dict(change("fix"), release_as="1.0.0")] * 2, (0, 9, 4))


class Parsing(unittest.TestCase):
    def test_conventional_title(self):
        m = release.CONVENTIONAL.match("feat(cli)!: add thing")
        self.assertEqual((m["type"], m["scope"], m["bang"], m["desc"]), ("feat", "cli", "!", "add thing"))
        self.assertIsNone(release.CONVENTIONAL.match("Add Phase 12 bundling"))

    def test_merge_commit_uses_body_title_not_subject(self):
        self.assertEqual(
            release.title_of("Merge pull request #7 from a/b", "\nfix: real title\n", 2), "fix: real title"
        )
        self.assertEqual(release.title_of("fix: squashed", "", 1), "fix: squashed")

    def test_release_commits_and_non_conventional_are_skipped(self):
        commits = [
            ("chore(release): v1.0.0 [skip ci]", "", 1, "a" * 40),
            ("Merge pull request #1 from a/b", "Tidy things up", 2, "b" * 40),
            ("Merge pull request #2 from a/c", "fix: real", 2, "c" * 40),
        ]
        changes, ignored = release.parse_changes(commits)
        self.assertEqual([c["type"] for c in changes], ["fix"])
        self.assertEqual(changes[0]["pr"], "2")
        self.assertEqual(len(ignored), 1)

    def test_breaking_change_footer(self):
        changes, _ = release.parse_changes([("feat: x", "BREAKING CHANGE: gone", 1, "d" * 40)])
        self.assertTrue(changes[0]["breaking"])

    def test_release_as_trailer_is_read_from_commit_body(self):
        changes, _ = release.parse_changes([
            ("Merge pull request #4 from a/b", "chore: prepare stable release\n\nRelease-As: 1.0.0\n", 2, "e" * 40)
        ])
        self.assertEqual(changes[0]["release_as"], "1.0.0")
        self.assertEqual(release.release_version(changes, (0, 9, 4)), (1, 0, 0))

    def test_multiple_trailers_fail(self):
        with self.assertRaises(SystemExit):
            release.parse_changes([("chore: release", "Release-As: 1.0.0\nRelease-As: 1.0.0", 1, "f" * 40)])


class Notes(unittest.TestCase):
    def test_sections_and_shared_changed_heading(self):
        changes = [change("feat"), change("perf"), change("refactor"), change("fix", breaking=True)]
        notes = release.render_notes("1.2.0", "v1.1.0", changes, "2026-01-01")
        self.assertEqual(notes.count("### Changed"), 1)
        self.assertLess(notes.index("### Breaking changes"), notes.index("### Added"))
        self.assertIn("compare/v1.1.0...v1.2.0", notes)


class Files(unittest.TestCase):
    def test_apply_rewrites_workspace_version_and_changelog(self):
        with tempfile.TemporaryDirectory() as d:
            root = pathlib.Path(d)
            (root / "Cargo.toml").write_text(
                '[workspace]\nmembers = ["a"]\n\n[workspace.package]\nversion = "0.0.4"\nedition = "2024"\n\n'
                '[package]\nname = "monitra"\nversion.workspace = true\n'
            )
            (root / "CHANGELOG.md").write_text("# Changelog\n\n## [Unreleased]\n\n## [0.0.4]\n")
            (root / "notes.md").write_text("## [0.0.5] - d\n")
            cwd = os.getcwd()
            os.chdir(d)
            try:
                self.assertEqual(release.read_package_version(), "0.0.4")
                release.cmd_apply(type("A", (), {"version": "0.0.5", "notes": "notes.md"}))
                self.assertEqual(release.read_package_version(), "0.0.5")
            finally:
                os.chdir(cwd)
            self.assertEqual((root / "Cargo.toml").read_text().count("0.0.5"), 1)
            self.assertIn("version.workspace = true", (root / "Cargo.toml").read_text())
            log = (root / "CHANGELOG.md").read_text()
            self.assertLess(log.index("[Unreleased]"), log.index("[0.0.5]"))
            self.assertLess(log.index("[0.0.5]"), log.index("[0.0.4]"))

    def test_rejects_non_semver(self):
        with self.assertRaises(ValueError):
            release.parse_version("1.2")


class Integration(unittest.TestCase):
    def run_git(self, *args):
        subprocess.run(["git", *args], cwd=self.dir, check=True, capture_output=True)

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.dir = self._tmp.name
        self._cwd = os.getcwd()
        self.run_git("init", "-q", "-b", "main")
        self.run_git("config", "user.email", "t@example.com")
        self.run_git("config", "user.name", "t")
        self.run_git("config", "commit.gpgsign", "false")
        os.chdir(self.dir)

    def tearDown(self):
        os.chdir(self._cwd)
        self._tmp.cleanup()

    def commit(self, msg):
        self.run_git("commit", "-q", "--allow-empty", "-m", msg)

    def merge_pr(self, number, title, branch_commit="wip", extra_body=None):
        self.run_git("checkout", "-q", "-b", f"pr{number}")
        self.commit(branch_commit)
        self.run_git("checkout", "-q", "main")
        message = ["-m", f"Merge pull request #{number} from a/pr{number}", "-m", title]
        if extra_body is not None:
            message.extend(["-m", extra_body])
        self.run_git("merge", "-q", "--no-ff", f"pr{number}", *message)

    def test_plan_reads_pr_titles_from_merge_commits_since_last_tag(self):
        self.commit("chore: init")
        self.run_git("tag", "v0.2.0")
        self.merge_pr(1, "fix: handle empty config", branch_commit="wip typo")
        self.merge_pr(2, "docs: reword readme")
        changes, ignored = release.parse_changes(release.read_commits("v0.2.0"))
        self.assertEqual([(c["type"], c["pr"]) for c in changes], [("docs", "2"), ("fix", "1")])
        self.assertEqual(ignored, [])
        self.assertEqual(release.decide_bump(changes, (0, 2, 0)), "patch")

    def test_last_release_prefers_highest_across_tag_schemes(self):
        self.commit("chore: init")
        self.run_git("tag", "monitra-v0.0.4")
        self.assertEqual(release.last_release(), ((0, 0, 4), "monitra-v0.0.4"))
        self.run_git("tag", "v0.1.0")
        self.assertEqual(release.last_release(), ((0, 1, 0), "v0.1.0"))

    def test_no_tag_means_none(self):
        self.commit("chore: init")
        self.assertIsNone(release.last_release())

    def test_merge_commit_trailer_releases_one_zero(self):
        self.commit("chore: init")
        self.run_git("tag", "v0.9.4")
        self.merge_pr(7, "feat!: stabilize public interfaces", extra_body="Release-As: 1.0.0")
        changes, ignored = release.parse_changes(release.read_commits("v0.9.4"))
        self.assertEqual(ignored, [])
        self.assertEqual(release.release_version(changes, (0, 9, 4)), (1, 0, 0))


if __name__ == "__main__":
    unittest.main()
