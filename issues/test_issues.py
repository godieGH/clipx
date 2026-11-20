"""Tests for issues.py:  python -m unittest discover -s issues"""
import contextlib
import io
import re
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import issues as T  # noqa: E402

README = "# Issues\n\n## Why this exists\n\ntext\n"


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.old = T.DIR
        T.DIR = self.tmp / "issues"
        T.DIR.mkdir()
        (T.DIR / "README.md").write_text(README, encoding="utf-8", newline="\n")
        (self.tmp / "README.md").write_text("# Root\n\n## Tracking issues\n", encoding="utf-8", newline="\n")

    def tearDown(self):
        T.DIR = self.old
        shutil.rmtree(self.tmp, ignore_errors=True)

    def run_cli(self, *argv):
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = T.main(list(argv))
        return code, out.getvalue(), err.getvalue()

    def report(self, title="Tray crash", body="It crashes.", *extra):
        code, out, err = self.run_cli("report", "--title", title, "--body", body, *extra)
        self.assertEqual(code, 0, err)
        return re.search(r"reported (\w+)", out)[1]

    def only(self, d):
        files = sorted(d.glob("*.md"))
        self.assertEqual(len(files), 1)
        return files[0]


class Records(Base):
    def test_front_matter_round_trip(self):
        meta = {"id": "a" * 40, "title": "T: with colon", "status": "report", "comment": ""}
        text = T.dump_record("issue", meta, "**Observation:** x")
        got, body = T.parse_record(text)
        self.assertEqual(got["title"], "T: with colon")
        self.assertNotIn("comment", got)
        self.assertEqual(body, "**Observation:** x")

    def test_parse_tolerates_crlf_and_rejects_missing_front_matter(self):
        meta, body = T.parse_record("---\r\nid: x\r\n---\r\n\r\nhello\r\n")
        self.assertEqual((meta["id"], body), ("x", "hello"))
        with self.assertRaises(T.Fail):
            T.parse_record("no front matter")

    def test_set_fields_touches_only_its_lines(self):
        text = "---\nid: a\nstatus: report\ncomment: old\n---\n\nbody **keep**\n"
        new = T.set_fields(text, {"status": "fixed", "weight": "weak", "comment": ""})
        self.assertEqual(new, "---\nid: a\nstatus: fixed\nweight: weak\n---\n\nbody **keep**\n")

    def test_set_fields_keeps_crlf(self):
        new = T.set_fields("---\r\nstatus: report\r\n---\r\nbody\r\n", {"status": "fixed"})
        self.assertEqual(new, "---\r\nstatus: fixed\r\n---\r\nbody\r\n")

    def test_slug_and_file_name(self):
        self.assertEqual(T.slugify("Hello, World!! 2", "x"), "hello-world-2")
        self.assertEqual(T.slugify("???", "issue"), "issue")
        self.assertLessEqual(len(T.slugify("word " * 30, "x")), 48)
        self.assertTrue(T.file_name("ab" * 20, "A title", "x").startswith("abababababab-a-title"))

    def test_new_id_shape_and_uniqueness(self):
        ids = []
        for _ in range(50):
            ids.append(T.new_id("same seed", ids))
        self.assertTrue(all(T.ID_RE.fullmatch(i) for i in ids))
        self.assertEqual(len({i[:T.FILE_LEN] for i in ids}), 50)


class Issues(Base):
    def test_report_creates_a_file_and_index(self):
        short = self.report("Tray crash", "It crashes.", "--weight", "weak", "--comment", "a | b")
        f = self.only(T.items_dir())
        meta, body = T.parse_record(T.read(f))
        self.assertTrue(f.name.startswith(meta["id"][:12] + "-tray-crash"))
        self.assertTrue(meta["id"].startswith(short))
        self.assertEqual((meta["status"], meta["weight"], body), ("report", "weak", "It crashes."))
        index = T.read(T.index_md())
        self.assertIn(f"[{short}](items/{f.name})", index)
        self.assertIn("report(weak)", index)
        self.assertIn("a \\| b", index)

    def test_status_update_and_prefix_resolution(self):
        short = self.report()
        self.assertEqual(self.run_cli("status", short[:5], "fixed", "--comment", "done")[0], 0)
        self.assertEqual(self.run_cli("update", short, "--text", "Retested on Android")[0], 0)
        meta, body = T.parse_record(T.read(self.only(T.items_dir())))
        self.assertEqual((meta["status"], meta["comment"]), ("fixed", "done"))
        self.assertIn("It crashes.\n\n**Update:** Retested on Android", body)
        self.assertIn("| fixed |", T.read(T.index_md()))

    def test_update_can_change_status(self):
        short = self.report()
        self.assertEqual(self.run_cli("update", short, "--text", "Fixed it", "--status", "fixed")[0], 0)
        self.assertEqual(T.parse_record(T.read(self.only(T.items_dir())))[0]["status"], "fixed")

    def test_referral_links_to_the_original(self):
        first = self.report("Original", "Body")
        code, _, err = self.run_cli("report", "--title", "Recheck", "--refers", first, "--body", "Still happens")
        self.assertEqual(code, 0, err)
        self.assertIn("↳ refers to", T.read(T.index_md()))
        self.assertEqual(self.run_cli("check")[0], 0)

    def test_bad_input_is_rejected(self):
        self.assertEqual(self.run_cli("report", "--title", "X", "--body", "b", "--weight", "huge")[0], 1)
        self.assertEqual(self.run_cli("report", "--title", "X", "--body", "b", "--refers", "nothere")[0], 1)
        self.assertEqual(self.run_cli("status", "zzzzz", "fixed")[0], 1)
        self.assertFalse(list(T.items_dir().glob("*.md")) if T.items_dir().exists() else [])

    def test_dry_run_writes_nothing(self):
        code, out, _ = self.run_cli("report", "--title", "X", "--body", "b", "--dry-run")
        self.assertEqual(code, 0)
        self.assertIn("dry run", out)
        self.assertFalse(T.items_dir().exists())

    def test_legacy_numbers_resolve(self):
        short = self.report()
        f = self.only(T.items_dir())
        T.write(f, T.set_fields(T.read(f), {"legacy": "0004"}))
        self.assertEqual(self.run_cli("status", "0004", "fixed")[0], 0)
        self.assertEqual(T.parse_record(T.read(f))[0]["status"], "fixed")
        self.assertTrue(T.read(T.index_md()).count("(was 0004)"))
        self.assertTrue(short)

    def test_ambiguous_and_short_refs(self):
        a = {"kind": "issue", "meta": {"id": "abcde" + "0" * 35}}
        b = {"kind": "issue", "meta": {"id": "abcde" + "1" * 35}}
        with self.assertRaises(T.Fail):
            T.resolve("abcde", [a, b], "issue")
        with self.assertRaises(T.Fail):
            T.resolve("abcd", [a, b], "issue")  # fewer than 5 characters
        self.assertIs(T.resolve("abcde1", [a, b], "issue"), b)
        self.assertEqual(T.short(a, [a, b]), "abcde00")


class Scenarios(Base):
    def test_add_and_set(self):
        iss = self.report()
        code, _, err = self.run_cli("wh", "add", "When A and B pair at once, what happens?",
                                    "--area", "pairing", "--note", "Both press Pair")
        self.assertEqual(code, 0, err)
        s = self.only(T.scen_dir())
        meta, body = T.parse_record(T.read(s))
        self.assertEqual((meta["status"], meta["area"], body), ("untested", "pairing", "Both press Pair"))
        self.assertEqual(self.run_cli("wh", "set", meta["id"][:6], "issue")[0], 1)  # needs an issue
        self.assertEqual(self.run_cli("wh", "set", meta["id"][:6], "issue", "--issue", iss)[0], 0)
        wh = T.read(T.wh_md())
        self.assertIn(f'<a id="wh-{meta["id"][:12]}"></a>', wh)
        self.assertIn("`issue`", wh)
        self.assertIn(f"(what_happens.md#wh-{meta['id'][:12]})", T.read(T.index_md()))
        self.assertEqual(self.run_cli("check")[0], 0)

    def test_bad_area_and_missing_question(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.run_cli("wh", "add", "Q?", "--area", "nowhere")  # argparse rejects unknown choices
        self.assertEqual(self.run_cli("wh", "add", " ", "--area", "ui")[0], 1)


class Check(Base):
    def test_clean_state_passes(self):
        self.report()
        code, out, err = self.run_cli("check")
        self.assertEqual((code, err), (0, ""))
        self.assertIn("1 issues", out)

    def test_stale_generated_file_fails_until_render(self):
        self.report()
        T.write(T.index_md(), T.read(T.index_md()) + "hand edit\n")
        self.assertEqual(self.run_cli("check")[0], 1)
        self.assertEqual(self.run_cli("render")[0], 0)
        self.assertEqual(self.run_cli("check")[0], 0)

    def test_crlf_generated_file_still_matches(self):
        self.report()
        T.write(T.index_md(), T.read(T.index_md()).replace("\n", "\r\n"))
        self.assertEqual(self.run_cli("check")[0], 0)

    def test_invalid_records_are_reported(self):
        self.report()
        f = self.only(T.items_dir())
        T.write(f, T.set_fields(T.read(f), {"status": "wip"}))
        code, _, err = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn("status 'wip'", err)

    def test_file_name_must_match_id(self):
        self.report()
        f = self.only(T.items_dir())
        f.rename(f.with_name("deadbeef0000-renamed.md"))
        self.assertIn("file name must start with", self.run_cli("check")[2])

    def test_duplicate_id_prefix_is_reported(self):
        self.report()
        f = self.only(T.items_dir())
        twin = T.items_dir() / (f.name[:12] + "-twin.md")
        twin.write_bytes(f.read_bytes())
        self.assertIn("must differ within their first", self.run_cli("check")[2])

    def test_unparseable_file_is_an_error_not_a_crash(self):
        self.report()
        (T.items_dir() / "000000000000-junk.md").write_text("no front matter", encoding="utf-8")
        code, _, err = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn("front matter not found", err)

    def test_broken_links_are_reported(self):
        self.report()
        T.write(T.DIR / "README.md", README + "\n[gone](items/nope.md) [bad](#nothing) [ok](#why-this-exists)\n"
                "[sec](../README.md#tracking-issues) [bad2](../README.md#missing)\n```\n[skipped](nowhere.md)\n```\n")
        code, _, err = self.run_cli("check")
        self.assertEqual(code, 1)
        self.assertIn("items/nope.md", err)
        self.assertIn("#nothing", err)
        self.assertIn("#missing", err)
        self.assertNotIn("nowhere.md", err)
        self.assertNotIn("#why-this-exists", err)
        self.assertNotIn("#tracking-issues", err)

    def test_root_readme_links_into_issues_are_checked(self):
        self.report()
        (self.tmp / "README.md").write_text("[a](issues/INDEX.md) [b](issues/README.md#nope) [c](other/x.md)\n",
                                            encoding="utf-8")
        err = self.run_cli("check")[2]
        self.assertIn("#nope", err)
        self.assertNotIn("other/x.md", err)
        self.assertNotIn("INDEX.md)", err)

    def test_github_slugs(self):
        self.assertEqual(T.gh_slug("What happens? scenarios"), "what-happens-scenarios")
        self.assertEqual(T.gh_slug("Change status, add an update, edit"), "change-status-add-an-update-edit")
        self.assertEqual(T.gh_slug("The `check` [tool](x.md)"), "the-check-tool")


class Editor(unittest.TestCase):
    def test_split_command_windows_quotes(self):
        argv = T.split_command('"C:\\Program Files\\Code\\Code.exe" --wait', windows=True)
        self.assertEqual(argv, ["C:\\Program Files\\Code\\Code.exe", "--wait"])
        self.assertEqual(T.split_command("code --wait", windows=False), ["code", "--wait"])

    def test_parse_form(self):
        fields, body = T.parse_form("<!-- hint -->\ntitle: T\nweight:\n---\nline one\nline two\n")
        self.assertEqual((fields["title"], fields["weight"], body), ("T", "", "line one\nline two"))
        with self.assertRaises(T.Fail):
            T.parse_form("title: T\nno separator")


if __name__ == "__main__":
    unittest.main()
