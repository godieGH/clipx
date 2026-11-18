"""Tests for issues.py.  Run:  python -m unittest discover -s issues -v   (from the repo root)"""
import contextlib
import difflib
import io
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import issues  # noqa: E402

FIXTURE = """# issues
intro

## The Table
> note

| issues | status | comment |
|--------|--------|---------|
| [0001](#0001) | fixed | first |
| [0002](#0002) | report(weak)| second |
| [0003](#0001) | fixed | referral to 1 |

**issues** = trailing text after the table

## 0001
**Observation:** one


## 0002
**Observation:** two
"""

FAKE_EDITOR = """import os, sys
t = os.environ.get("FAKE_TEXT")
if t is not None:
    open(sys.argv[1], "w", encoding="utf-8", newline="").write(t)
"""


def removed_lines(old, new):
    d = list(difflib.unified_diff(old.splitlines(True), new.splitlines(True), n=0))
    return [l for l in d[2:] if l.startswith("-")], [l for l in d[2:] if l.startswith("+")]


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="issues-test-"))
        self.old_dir, self.old_tmpdir = issues.DIR, tempfile.tempdir
        (self.tmp / "tmpdir").mkdir()
        tempfile.tempdir = str(self.tmp / "tmpdir")
        issues.DIR = self.tmp
        self.env = {k: os.environ.get(k) for k in ("ISSUES_EDITOR", "FAKE_TEXT", "EDITOR", "VISUAL")}
        for k in self.env:
            os.environ.pop(k, None)
        self.write_issue(FIXTURE)

    def tearDown(self):
        issues.DIR = self.old_dir
        tempfile.tempdir = self.old_tmpdir
        for k, v in self.env.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v
        shutil.rmtree(self.tmp, ignore_errors=True)

    def write_issue(self, text):
        (self.tmp / "issue.md").write_bytes(text.encode("utf-8"))

    def read_issue(self):
        return (self.tmp / "issue.md").read_bytes().decode("utf-8")

    def run_cli(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = issues.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def use_fake_editor(self, text=None):
        script = self.tmp / "fake_editor.py"
        script.write_text(FAKE_EDITOR, encoding="utf-8")
        os.environ["ISSUES_EDITOR"] = f'"{sys.executable}" "{script}"'
        if text is None:
            os.environ.pop("FAKE_TEXT", None)
        else:
            os.environ["FAKE_TEXT"] = text


class ReportTests(Base):
    def test_noninteractive_appends_row_and_section_only(self):
        code, out, err = self.run_cli("report", "--comment", "third one", "--body", "three", "--weight", "strong")
        self.assertEqual(code, 0, err)
        new = self.read_issue()
        gone, added = removed_lines(FIXTURE, new)
        self.assertEqual(gone, [], "nothing existing may be removed or changed")
        self.assertIn("+| [0004](#0004) | report(strong) | third one |\n", added)
        self.assertTrue(new.endswith("\n## 0004\n**Observation:** three\n"))
        # row sits directly under the last table row, before the trailing text
        lines = new.splitlines()
        self.assertEqual(lines[lines.index("| [0003](#0001) | fixed | referral to 1 |") + 1],
                         "| [0004](#0004) | report(strong) | third one |")

    def test_crlf_file_stays_crlf(self):
        self.write_issue(FIXTURE.replace("\n", "\r\n"))
        code, _, err = self.run_cli("report", "--comment", "c", "--body", "line1\nline2")
        self.assertEqual(code, 0, err)
        raw = self.read_issue()
        self.assertEqual(raw.count("\n"), raw.count("\r\n"), "no bare LF may appear")
        self.assertIn("| [0004](#0004) | report | c |\r\n", raw)
        self.assertTrue(raw.endswith("**Observation:** line1\r\nline2\r\n"))

    def test_file_without_trailing_newline(self):
        self.write_issue(FIXTURE.rstrip("\n"))
        code, _, err = self.run_cli("report", "--comment", "c", "--body", "b")
        self.assertEqual(code, 0, err)
        self.assertEqual(self.run_cli("check")[0], 0)

    def test_referral_adds_row_without_section(self):
        code, _, err = self.run_cli("report", "--refers", "0002", "--comment", "same as 2, again")
        self.assertEqual(code, 0, err)
        new = self.read_issue()
        self.assertIn("| [0004](#0002) | report | same as 2, again |\n", new)
        self.assertNotIn("## 0004", new)
        self.assertEqual(self.run_cli("check")[0], 0)

    def test_referral_of_referral_resolves_to_real_section(self):
        self.run_cli("report", "--refers", "0003", "--comment", "x")
        self.assertIn("| [0004](#0001) |", self.read_issue())

    def test_pipe_in_comment_is_escaped(self):
        self.run_cli("report", "--comment", "a | b", "--body", "x")
        self.assertIn("| a \\| b |", self.read_issue())

    def test_bad_weight_and_status_rejected(self):
        self.assertEqual(self.run_cli("report", "--comment", "c", "--body", "b", "--weight", "huge")[0], 1)
        self.assertEqual(self.run_cli("report", "--comment", "c", "--body", "b", "--status", "wip")[0], 1)
        self.assertEqual(self.read_issue(), FIXTURE)

    def test_unknown_referral_rejected(self):
        self.assertEqual(self.run_cli("report", "--refers", "0099", "--comment", "c")[0], 1)

    def test_dry_run_writes_nothing(self):
        code, out, _ = self.run_cli("report", "--comment", "c", "--body", "b", "--dry-run")
        self.assertEqual(code, 0)
        self.assertIn("+| [0004](#0004)", out)
        self.assertEqual(self.read_issue(), FIXTURE)


class EditorFlowTests(Base):
    def test_editor_flow_appends(self):
        self.use_fake_editor("status: report\nweight: weak\nrefers:\ncomment: from editor\n---\nbody from editor\n")
        code, _, err = self.run_cli("report")
        self.assertEqual(code, 0, err)
        new = self.read_issue()
        self.assertIn("| [0004](#0004) | report(weak) | from editor |", new)
        self.assertTrue(new.endswith("**Observation:** body from editor\n"))

    def test_unchanged_draft_aborts(self):
        self.use_fake_editor(None)
        code, out, _ = self.run_cli("report")
        self.assertEqual(code, 0)
        self.assertIn("nothing to report", out)
        self.assertEqual(self.read_issue(), FIXTURE)

    def test_html_comment_lines_are_dropped(self):
        self.use_fake_editor("<!-- hi -->\nstatus: report\nweight:\nrefers:\ncomment: c\n---\n<!-- note -->\nreal body\n")
        self.assertEqual(self.run_cli("report")[0], 0)
        self.assertNotIn("<!--", self.read_issue()[len(FIXTURE):])

    def test_failed_validation_keeps_draft_and_resume_works(self):
        self.use_fake_editor("status: report\nweight:\nrefers:\ncomment:\n---\nbody\n")
        code, _, err = self.run_cli("report")
        self.assertEqual(code, 1)
        self.assertIn("--resume", err)
        self.assertEqual(self.read_issue(), FIXTURE)
        draft = issues.latest_draft("report")
        draft.write_text("status: report\nweight:\nrefers:\ncomment: fixed it\n---\nbody\n", encoding="utf-8")
        code, _, err = self.run_cli("report", "--resume")
        self.assertEqual(code, 0, err)
        self.assertIn("| fixed it |", self.read_issue())
        self.assertFalse(draft.exists())

    def test_gui_editor_gets_wait_flag(self):
        # on Windows an extensionless file is not executable: use code.cmd (found via PATHEXT)
        fake = self.tmp / ("code.cmd" if os.name == "nt" else "code")
        fake.write_text("#!/bin/sh\n", encoding="utf-8")
        fake.chmod(0o755)
        os.environ["ISSUES_EDITOR"] = str(fake)
        self.assertEqual(issues.editor_argv(), [shutil.which(str(fake)), "--wait"])

    def test_split_command_windows_strips_quotes_and_keeps_backslashes(self):
        raw = '"C:\\Program Files\\Microsoft VS Code\\Code.exe" --wait'
        self.assertEqual(issues.split_command(raw, windows=True),
                         ["C:\\Program Files\\Microsoft VS Code\\Code.exe", "--wait"])
        self.assertEqual(issues.split_command("code --wait", windows=True), ["code", "--wait"])
        self.assertEqual(issues.split_command("'my editor' -x", windows=True), ["my editor", "-x"])

    def test_split_command_posix(self):
        self.assertEqual(issues.split_command('"/opt/my editor/ed" -w', windows=False), ["/opt/my editor/ed", "-w"])


class StatusUpdateTests(Base):
    def test_status_changes_only_that_row(self):
        code, _, err = self.run_cli("status", "0002", "fixed")
        self.assertEqual(code, 0, err)
        gone, added = removed_lines(FIXTURE, self.read_issue())
        self.assertEqual(gone, ["-| [0002](#0002) | report(weak)| second |\n"])
        self.assertEqual(added, ["+| [0002](#0002) | fixed| second |\n"])

    def test_status_with_comment(self):
        self.run_cli("status", "0002", "fixed", "--comment", "done in abc123")
        self.assertIn("| [0002](#0002) | fixed| done in abc123 |\n", self.read_issue())

    def test_status_validation(self):
        self.assertEqual(self.run_cli("status", "0002", "wip")[0], 1)
        self.assertEqual(self.run_cli("status", "0009", "fixed")[0], 1)

    def test_update_goes_inside_section_before_next_heading(self):
        code, _, err = self.run_cli("update", "0001", "--text", "later note")
        self.assertEqual(code, 0, err)
        lines = self.read_issue().splitlines()
        u, h1, h2 = lines.index("**Update:** later note"), lines.index("## 0001"), lines.index("## 0002")
        self.assertTrue(h1 < u < h2)
        self.assertEqual(lines[h2 - 1], "")
        gone, _ = removed_lines(FIXTURE, self.read_issue())
        self.assertEqual(gone, [])

    def test_update_last_section_and_status_flag(self):
        code, _, err = self.run_cli("update", "0002", "--text", "x", "--status", "fixed")
        self.assertEqual(code, 0, err)
        new = self.read_issue()
        self.assertTrue(new.endswith("**Observation:** two\n\n**Update:** x\n"))
        self.assertIn("| fixed| second |", new)

    def test_update_on_referral_is_rejected(self):
        code, _, err = self.run_cli("update", "0003", "--text", "x")
        self.assertEqual(code, 1)
        self.assertIn("referral to 0001", err)

    def test_update_dates_config(self):
        (self.tmp / "config.local.toml").write_text("[report]\nupdate_dates = true\n", encoding="utf-8")
        self.run_cli("update", "0001", "--text", "dated")
        self.assertRegex(self.read_issue(), r"\*\*Update \(\d{4}-\d{2}-\d{2}\):\*\* dated")

    def test_update_via_editor(self):
        self.use_fake_editor("<!-- c -->\n\nedited update\nsecond line\n")
        self.assertEqual(self.run_cli("update", "0001")[0], 0)
        self.assertIn("**Update:** edited update\nsecond line\n", self.read_issue())

    def test_list_and_next(self):
        self.assertEqual(self.run_cli("next")[1].strip(), "0004")
        out = self.run_cli("list", "--status", "report")[1]
        self.assertIn("0002", out)
        self.assertNotIn("0001", out)


class CheckTests(Base):
    def test_fixture_is_clean(self):
        code, out, err = self.run_cli("check")
        self.assertEqual(code, 0, err)
        self.assertIn("3 issues", out)

    def test_missing_section(self):
        self.write_issue(FIXTURE.replace("## 0002\n", "## 0009\n"))
        code, _, err = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn("no '## 0002' section", err)
        self.assertIn("'## 0009' has no row", err)

    def test_gap_in_numbers(self):
        self.write_issue(FIXTURE.replace("[0002](#0002)", "[0005](#0005)").replace("## 0002", "## 0005"))
        self.assertIn("0001..0003 in order", self.run_cli("check")[2])

    def test_bad_status_and_weight(self):
        self.write_issue(FIXTURE.replace("report(weak)", "report(huge)").replace("| fixed | first", "| wip | first"))
        err = self.run_cli("check")[2]
        self.assertIn("weight 'huge'", err)
        self.assertIn("'wip' must start with", err)

    def test_dangling_referral(self):
        self.write_issue(FIXTURE.replace("[0003](#0001)", "[0003](#0007)"))
        self.assertIn("#0007", self.run_cli("check")[2])


class WhTests(Base):
    def test_add_set_render_and_stale_detection(self):
        (self.tmp / "what_happens.toml").write_text("", encoding="utf-8")
        code, _, err = self.run_cli("wh", "add", "When X happens, what happens?", "--area", "ui", "--issue", "0002", "--note", "n")
        self.assertEqual(code, 0, err)
        md = (self.tmp / "what_happens.md").read_text(encoding="utf-8")
        self.assertIn("**WH-001** `untested`: When X happens, what happens?", md)
        self.assertIn("[0002](issue.md#0002)", md)
        self.assertIn("| [0003](issue.md#0001) (see 0001) | fixed | none |", md)
        self.assertEqual(self.run_cli("check")[0], 0)
        self.assertEqual(self.run_cli("wh", "set", "WH-001", "tested")[0], 0)
        self.assertIn("`tested`", (self.tmp / "what_happens.md").read_text(encoding="utf-8"))
        (self.tmp / "what_happens.md").write_text("hand edited\n", encoding="utf-8")
        code, _, err = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn("out of date", err)
        self.assertEqual(self.run_cli("wh", "render")[0], 0)
        self.assertEqual(self.run_cli("check")[0], 0)

    def test_status_issue_requires_issue_number(self):
        (self.tmp / "what_happens.toml").write_text("", encoding="utf-8")
        self.run_cli("wh", "add", "q?", "--area", "ui")
        code, _, err = self.run_cli("wh", "set", "WH-001", "issue")
        self.assertEqual(code, 1)
        self.assertIn("needs an issue number", err)
        self.assertEqual(self.run_cli("wh", "set", "WH-001", "issue", "--issue", "0002")[0], 0)

    def test_unknown_issue_link_rejected(self):
        (self.tmp / "what_happens.toml").write_text("", encoding="utf-8")
        self.assertEqual(self.run_cli("wh", "add", "q?", "--area", "ui", "--issue", "0099")[0], 1)

    def test_add_via_editor(self):
        (self.tmp / "what_happens.toml").write_text("", encoding="utf-8")
        self.use_fake_editor("area: pairing\nissues: 0001, 0002\nnote: hint\n---\nWhen A\nand B pair?\n")
        code, _, err = self.run_cli("wh", "add")
        self.assertEqual(code, 0, err)
        out = self.run_cli("wh", "list", "--area", "pairing")[1]
        self.assertIn("When A and B pair?", out)


class RealFileTests(unittest.TestCase):
    """The real issues/issue.md must pass the checks and survive a dry run untouched."""

    def test_real_issue_md(self):
        real = Path(__file__).resolve().parent / "issue.md"
        if not real.exists():
            self.skipTest("no issue.md")
        lines = issues.split_lines(real.read_bytes().decode("utf-8"))
        self.assertEqual(issues.check_issues(lines, issues.config()), [])
        new, num = issues.apply_report(lines, {"comment": "probe"}, "body", issues.config())
        self.assertEqual("".join(new[:len(lines)]).count("\n") >= "".join(lines).count("\n"), True)
        self.assertEqual(len(num), 4)


if __name__ == "__main__":
    unittest.main()
