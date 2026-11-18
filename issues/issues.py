#!/usr/bin/env python3
"""issues: one tool for everything under issues/   (python >= 3.11, stdlib only)

issue.md stays hand-written. This tool only appends to it, or edits a single table row,
and leaves every other byte alone (line endings included).

  issues.py report                  open your editor on a temp draft, then append table row + section
  issues.py report --comment "..." --body "..." [--weight weak] [--status report]   no editor
  issues.py report --refers 0002 --comment "..."   referral row (links to #0002, no section)
  issues.py report --resume         reopen the last draft that failed validation
  issues.py update NNNN [--text "..."] [--status fixed]   append an **Update:** block to a section
  issues.py status NNNN fixed [--comment "..."]            edit only that table row
  issues.py list [--status S]       print the table
  issues.py next                    print the next free issue number
  issues.py check                   validate issue.md, what_happens.toml and the generated .md

  issues.py wh add ["question?"] [--area A] [--issue NNNN] [--note T]
  issues.py wh set WH-003 tested|untested|issue [--issue NNNN]
  issues.py wh list [--status S] [--area A]
  issues.py wh render               regenerate what_happens.md (add/set do this too)

Any write command takes --dry-run (print a diff, write nothing).
Config: issues/config.toml (shared) and issues/config.local.toml (yours, gitignored).
"""
import argparse
import difflib
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

DIR = Path(os.environ.get("ISSUES_DIR") or Path(__file__).resolve().parent)

DEFAULTS = {
    "report": {
        "default_status": "report",
        "statuses": ["report", "fixed"],
        "weights": ["weak", "medium", "strong"],
        "update_dates": False,
    },
    "editor": {"command": ""},
    "wh": {
        "areas": ["pairing", "connect", "transfer", "clipboard", "discovery", "protocol", "ui", "platform"],
        "statuses": ["untested", "tested", "issue"],
    },
}

GUI_WAIT = {
    "code": "--wait", "code-insiders": "--wait", "cursor": "--wait", "codium": "--wait",
    "windsurf": "--wait", "subl": "--wait", "zed": "--wait", "atom": "--wait", "mate": "-w",
}


class Fail(Exception):
    pass


# ----------------------------------------------------------------------------- paths / config

def issue_md():
    return DIR / "issue.md"


def wh_toml():
    return DIR / "what_happens.toml"


def wh_md():
    return DIR / "what_happens.md"


def drafts_dir():
    d = Path(tempfile.gettempdir()) / "clipx-issues-drafts"
    d.mkdir(parents=True, exist_ok=True)
    return d


def config():
    cfg = {k: dict(v) for k, v in DEFAULTS.items()}
    for name in ("config.toml", "config.local.toml"):
        p = DIR / name
        if p.exists():
            for sec, vals in tomllib.loads(p.read_text(encoding="utf-8")).items():
                cfg.setdefault(sec, {}).update(vals)
    return cfg


# ----------------------------------------------------------------------------- file io (byte-safe)

def split_lines(text):
    return re.findall(r"[^\n]*\n|[^\n]+", text)


def read_lines(path):
    return split_lines(path.read_bytes().decode("utf-8"))


def detect_eol(lines):
    crlf = sum(1 for l in lines if l.endswith("\r\n"))
    return "\r\n" if lines and crlf * 2 > len(lines) else "\n"


def ensure_nl(lines, eol):
    if lines and not lines[-1].endswith("\n"):
        lines[-1] += eol


def commit(path, old, new, dry):
    if dry:
        sys.stdout.writelines(difflib.unified_diff(old, new, f"a/{path.name}", f"b/{path.name}"))
        print("(dry run: nothing written)")
        return
    stamp = time.strftime("%Y%m%d-%H%M%S")
    if path.exists():
        shutil.copy2(path, drafts_dir() / f"{path.name}.{stamp}.bak")
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_bytes("".join(new).encode("utf-8"))
    os.replace(tmp, path)


# ----------------------------------------------------------------------------- issue.md model

ROW = re.compile(r"^\|\s*\[(\d{4})\]\(#(\d{4})\)\s*\|([^|]*)\|(.*)$")
HEAD = re.compile(r"^## (\d{4})\s*$")


def table_span(lines):
    start = next((i for i, l in enumerate(lines) if re.match(r"^\|\s*issues\s*\|", l)), None)
    if start is None:
        raise Fail("issue.md: table header '| issues | status | comment |' not found")
    end = start + 1
    while end < len(lines) and lines[end].startswith("|"):
        end += 1
    return start, end


def rows(lines):
    s, e = table_span(lines)
    out = []
    for i in range(s + 2, e):
        m = ROW.match(lines[i])
        if m:
            out.append({"num": m[1], "anchor": m[2], "status": m[3].strip(),
                        "comment": m[4].strip().rstrip("|").strip(), "i": i})
    return out


def headings(lines):
    return {m[1]: i for i, l in enumerate(lines) if (m := HEAD.match(l))}


def status_base(status):
    return re.split(r"[(/\s]", status, maxsplit=1)[0]


def anchor_of(lines, num):
    for r in rows(lines):
        if r["num"] == num:
            return r["anchor"]
    return num


def issue_link(lines, num):
    a = anchor_of(lines, num)
    return f"[{num}](issue.md#{a})" if a in headings(lines) else num


def check_issues(lines, cfg):
    errs = []
    rs = rows(lines)
    heads = headings(lines)
    nums = [r["num"] for r in rs]
    if nums != [f"{i:04d}" for i in range(1, len(nums) + 1)]:
        errs.append(f"issue.md: table numbers must be 0001..{len(nums):04d} in order, got {', '.join(nums)}")
    known = set(nums)
    allowed = cfg["report"]["statuses"]
    weights = cfg["report"]["weights"]
    for r in rs:
        n = r["num"]
        if r["anchor"] not in known:
            errs.append(f"{n}: table link points to #{r['anchor']}, which is not an issue in the table")
        elif r["anchor"] not in heads:
            errs.append(f"{n}: table link points to #{r['anchor']}, but there is no '## {r['anchor']}' section")
        base = status_base(r["status"])
        if base not in allowed:
            errs.append(f"{n}: status {r['status']!r} must start with one of {', '.join(allowed)}")
        w = re.search(r"\(([^)]*)\)", r["status"])
        if w and weights and w[1] not in weights:
            errs.append(f"{n}: weight {w[1]!r} is not one of {', '.join(weights)}")
    for h in heads:
        if h not in known:
            errs.append(f"issue.md: '## {h}' has no row in the table")
    return errs


# ----------------------------------------------------------------------------- editor + drafts

def split_command(raw, windows=None):
    """Split an editor command. On Windows shlex keeps the quotes (posix=False), so strip them."""
    windows = (os.name == "nt") if windows is None else windows
    argv = shlex.split(raw, posix=not windows)
    if windows:
        argv = [t[1:-1] if len(t) >= 2 and t[0] == t[-1] and t[0] in "\"'" else t for t in argv]
    return argv


def editor_argv():
    cfg = config()
    raw = (os.environ.get("ISSUES_EDITOR") or cfg["editor"].get("command")
           or os.environ.get("VISUAL") or os.environ.get("EDITOR"))
    if raw:
        argv = split_command(raw)
    else:
        cands = ["code", "cursor", "codium", "subl", "nano", "vim", "vi"] + (["notepad"] if os.name == "nt" else [])
        argv = next(([c] for c in cands if shutil.which(c)), None)
        if not argv:
            raise Fail("no editor found. Set one in issues/config.local.toml ([editor] command = \"nano\"), "
                       "or export EDITOR, or pass the text with flags (--comment/--body)")
    exe = shutil.which(argv[0])
    if not exe:
        raise Fail(f"editor {argv[0]!r} not found on PATH")
    argv[0] = exe
    flag = GUI_WAIT.get(Path(exe).stem.lower())
    if flag and flag not in argv:
        argv.append(flag)
    return argv


def open_editor(path):
    argv = editor_argv()
    r = subprocess.run(argv + [str(path)])
    if r.returncode != 0:
        raise Fail(f"editor exited with code {r.returncode}; draft kept at {path}")


def new_draft(kind, text):
    p = drafts_dir() / f"{kind}-{time.strftime('%Y%m%d-%H%M%S')}-{os.getpid()}.md"
    p.write_text(text, encoding="utf-8", newline="\n")
    return p


def latest_draft(kind):
    ds = sorted(drafts_dir().glob(f"{kind}-*.md"), key=lambda p: p.stat().st_mtime)
    if not ds:
        raise Fail(f"no saved {kind} draft to resume")
    return ds[-1]


COMMENT_LINE = re.compile(r"^\s*<!--.*-->\s*$")


def parse_form(text, header=True):
    lines = [l for l in text.replace("\r\n", "\n").split("\n") if not COMMENT_LINE.match(l)]
    fields = {}
    if header:
        stripped = [l.strip() for l in lines]
        if "---" not in stripped:
            raise Fail("the '---' line between the fields and the text is missing")
        k = stripped.index("---")
        for l in lines[:k]:
            if not l.strip():
                continue
            m = re.match(r"^(\w+)\s*:\s*(.*)$", l)
            if not m:
                raise Fail(f"bad field line: {l!r}")
            fields[m[1].lower()] = m[2].strip()
        lines = lines[k + 1:]
    return fields, "\n".join(lines).strip()


# ----------------------------------------------------------------------------- issue.md edits

REPORT_TEMPLATE = """\
<!-- New issue. Whole-line HTML comments like this one are dropped. Save and close the editor to submit. -->
<!-- comment = one line for the table. The text below '---' becomes the '## NNNN' section. -->
<!-- Referral to another issue: set 'refers: 0002' and leave the text empty. Close without edits to abort. -->
status: {status}
weight: {weight}
refers: {refers}
comment: {comment}
---
{body}
"""

UPDATE_TEMPLATE = """\
<!-- Update for issue {num}. Whole-line HTML comments are dropped. Save and close to submit. -->
<!-- Changing the status too? Use:  issues.py update {num} --status fixed -->

"""


def apply_report(lines, f, body, cfg):
    rc = cfg["report"]
    status = f.get("status") or rc["default_status"]
    weight = f.get("weight", "")
    if status_base(status) not in rc["statuses"]:
        raise Fail(f"status {status!r} must start with one of {', '.join(rc['statuses'])}")
    if weight and weight not in rc["weights"]:
        raise Fail(f"weight {weight!r} must be one of {', '.join(rc['weights'])}")
    comment = " ".join(f.get("comment", "").split()).replace("|", "\\|")
    if not comment:
        raise Fail("comment is empty (it is the one-line summary shown in the table)")
    rs = rows(lines)
    known = {r["num"]: r for r in rs}
    heads = headings(lines)
    refers = f.get("refers", "")
    if refers:
        if refers not in known:
            raise Fail(f"refers: {refers} is not an issue in the table")
        anchor = known[refers]["anchor"]
        if anchor not in heads:
            raise Fail(f"refers: {refers} has no section to point to")
    elif not body:
        raise Fail("the text below '---' is empty")
    num = f"{max([int(r['num']) for r in rs] or [0]) + 1:04d}"
    cell = status if "(" in status or not weight else f"{status}({weight})"
    new = list(lines)
    ensure_nl(new, detect_eol(lines))
    eol = detect_eol(lines)
    s, e = table_span(new)
    if not new[e - 1].endswith("\n"):
        new[e - 1] += eol
    new.insert(e, f"| [{num}](#{refers and known[refers]['anchor'] or num}) | {cell} | {comment} |{eol}")
    if not refers:
        parts = body.replace("\r\n", "\n").split("\n")
        if not parts[0].startswith("**"):
            parts[0] = "**Observation:** " + parts[0]
        new += [eol, f"## {num}{eol}"] + [p + eol for p in parts]
    return new, num


def set_status(lines, num, status, comment, cfg):
    rc = cfg["report"]
    if status_base(status) not in rc["statuses"]:
        raise Fail(f"status {status!r} must start with one of {', '.join(rc['statuses'])}")
    w = re.search(r"\(([^)]*)\)", status)
    if w and rc["weights"] and w[1] not in rc["weights"]:
        raise Fail(f"weight {w[1]!r} must be one of {', '.join(rc['weights'])}")
    r = next((r for r in rows(lines) if r["num"] == num), None)
    if not r:
        raise Fail(f"issue {num} is not in the table")
    line = lines[r["i"]]
    m = ROW.match(line)
    raw = m[3]
    lead = len(raw) - len(raw.lstrip())
    trail = len(raw) - len(raw.rstrip())
    new3 = raw[:lead] + status + raw[len(raw) - trail:]
    out = line[:m.start(3)] + new3 + line[m.end(3):]
    if comment is not None:
        tail = re.search(r"\r?\n$", out)
        eol = tail[0] if tail else ""
        head = out[:m.start(3) + len(new3) + 1]
        out = head + f" {' '.join(comment.split()).replace('|', chr(92) + '|')} |" + eol
    new = list(lines)
    new[r["i"]] = out
    return new


def add_update(lines, num, text, cfg):
    if not text:
        raise Fail("update text is empty")
    r = next((r for r in rows(lines) if r["num"] == num), None)
    if not r:
        raise Fail(f"issue {num} is not in the table")
    if r["anchor"] != num:
        raise Fail(f"{num} is a referral to {r['anchor']}: update {r['anchor']} instead")
    heads = headings(lines)
    if num not in heads:
        raise Fail(f"issue {num} has no '## {num}' section")
    eol = detect_eol(lines)
    h = heads[num]
    nxt = min([i for i in heads.values() if i > h] or [len(lines)])
    pos = nxt
    while pos > h + 1 and lines[pos - 1].strip() == "":
        pos -= 1
    label = f"Update ({time.strftime('%Y-%m-%d')})" if cfg["report"]["update_dates"] else "Update"
    parts = text.replace("\r\n", "\n").split("\n")
    parts[0] = f"**{label}:** " + parts[0]
    new = list(lines)
    if not new[pos - 1].endswith("\n"):
        new[pos - 1] += eol
    new[pos:pos] = [eol] + [p + eol for p in parts]
    return new


# ----------------------------------------------------------------------------- commands: issues

def cmd_report(a):
    cfg = config()
    lines = read_lines(issue_md())
    draft = None
    form = {"status": a.status or "", "weight": a.weight or "", "refers": a.refers or "", "comment": a.comment or ""}
    body = a.body or ""
    if a.resume:
        draft = latest_draft("report")
        form, body = parse_form(draft.read_text(encoding="utf-8"))
    elif not (a.comment and (a.body or a.refers)):
        tpl = REPORT_TEMPLATE.format(status=form["status"] or cfg["report"]["default_status"],
                                     weight=form["weight"], refers=form["refers"],
                                     comment=form["comment"], body=body)
        draft = new_draft("report", tpl)
        open_editor(draft)
        text = draft.read_text(encoding="utf-8")
        if text.strip() == tpl.strip():
            draft.unlink()
            print("nothing to report (draft unchanged)")
            return 0
        form, body = parse_form(text)
    try:
        new, num = apply_report(lines, form, body, cfg)
    except Fail as e:
        if draft:
            raise Fail(f"{e}\n  draft kept at {draft}\n  continue with: issues.py report --resume")
        raise
    commit(issue_md(), lines, new, a.dry_run)
    if draft and not a.dry_run:
        draft.unlink(missing_ok=True)
    if not a.dry_run:
        print(f"reported {num}")
    return 0


def cmd_update(a):
    cfg = config()
    lines = read_lines(issue_md())
    text = a.text
    draft = None
    if text is None:
        tpl = UPDATE_TEMPLATE.format(num=a.num)
        draft = new_draft("update", tpl)
        open_editor(draft)
        raw = draft.read_text(encoding="utf-8")
        if raw.strip() == tpl.strip():
            draft.unlink()
            print("nothing to add (draft unchanged)")
            return 0
        _, text = parse_form(raw, header=False)
    try:
        new = add_update(lines, a.num, text, cfg)
        if a.status:
            new = set_status(new, a.num, a.status, None, cfg)
    except Fail as e:
        if draft:
            raise Fail(f"{e}\n  your text is kept at {draft}")
        raise
    commit(issue_md(), lines, new, a.dry_run)
    if draft and not a.dry_run:
        draft.unlink(missing_ok=True)
    if not a.dry_run:
        print(f"updated {a.num}")
    return 0


def cmd_status(a):
    lines = read_lines(issue_md())
    new = set_status(lines, a.num, a.status, a.comment, config())
    commit(issue_md(), lines, new, a.dry_run)
    if not a.dry_run:
        print(f"{a.num} -> {a.status}")
    return 0


def cmd_list(a):
    for r in rows(read_lines(issue_md())):
        if a.status and status_base(r["status"]) != a.status:
            continue
        ref = f" (see {r['anchor']})" if r["anchor"] != r["num"] else ""
        print(f"{r['num']}  {r['status']:<14} {r['comment'][:90]}{ref}")
    return 0


def cmd_next(a):
    rs = rows(read_lines(issue_md()))
    print(f"{max([int(r['num']) for r in rs] or [0]) + 1:04d}")
    return 0


def cmd_check(a):
    cfg = config()
    lines = read_lines(issue_md())
    errs = check_issues(lines, cfg)
    scenarios = load_wh()
    errs += check_wh(scenarios, lines, cfg)
    if wh_md().exists():
        have = wh_md().read_bytes().decode("utf-8").replace("\r\n", "\n")
        if have != render_wh(scenarios, lines, cfg):
            errs.append("what_happens.md is out of date: run `issues.py wh render`")
    if errs:
        print("\n".join(errs), file=sys.stderr)
        return 1
    rs = rows(lines)
    covered = {n for s in scenarios for n in s.get("issues", [])}
    open_no_wh = [r["num"] for r in rs if status_base(r["status"]) == "report" and r["num"] not in covered]
    print(f"ok: {len(rs)} issues, {len(scenarios)} scenarios")
    if open_no_wh:
        print("open issues with no what-happens scenario: " + ", ".join(open_no_wh))
    return 0


# ----------------------------------------------------------------------------- what_happens

WH_INTRO = """\
# What Happens?

> GENERATED FILE: do not edit by hand. Edit `what_happens.toml` or run `python issues/issues.py wh add`, then `python issues/issues.py wh render`.

> This is a repo way of asking what might happen if a certain behavior, action or event in this software is done or happens. What other expected or unexpected behaviors might be triggered. It does not answer what the triggered behaviors are; it lists the questions for what can trigger them. This lets users or devs reproduce and test on their own side, without a known reference behavior, so they can shape and report unexpected behaviors.

**How to use:** pick an `untested` scenario, try it on your devices, then report what you saw. If something unexpected happened, report it with `python issues/issues.py report` and link it here (`wh set WH-00N issue --issue NNNN`).
"""

WH_TEMPLATE = """\
<!-- New 'what happens' scenario. Whole-line HTML comments are dropped. Save and close to submit. -->
<!-- Write one question below '---', e.g. "When X happens, what happens?". A question only, never an answer. -->
area: {area}
issues: {issues}
note: {note}
---
{question}
"""


def load_wh():
    if not wh_toml().exists():
        return []
    return tomllib.loads(wh_toml().read_text(encoding="utf-8")).get("scenario", [])


def check_wh(scenarios, lines, cfg):
    errs, ids = [], set()
    known = {r["num"] for r in rows(lines)}
    areas, statuses = cfg["wh"]["areas"], cfg["wh"]["statuses"]
    for s in scenarios:
        sid = s.get("id", "<no id>")
        if not re.fullmatch(r"WH-\d{3,}", sid):
            errs.append(f"{sid}: id must look like WH-001")
        if sid in ids:
            errs.append(f"{sid}: duplicate id")
        ids.add(sid)
        if not str(s.get("question", "")).strip():
            errs.append(f"{sid}: empty question")
        if s.get("area") not in areas:
            errs.append(f"{sid}: area must be one of {', '.join(areas)}")
        if s.get("status") not in statuses:
            errs.append(f"{sid}: status must be one of {', '.join(statuses)}")
        refs = s.get("issues", [])
        for n in refs:
            if n not in known:
                errs.append(f"{sid}: issue {n} is not in the issue.md table")
        if s.get("status") == "issue" and not refs:
            errs.append(f"{sid}: status 'issue' needs an issue number")
    return errs


def render_wh(scenarios, lines, cfg):
    areas, statuses = cfg["wh"]["areas"], cfg["wh"]["statuses"]
    table = {r["num"]: r for r in rows(lines)}
    counts = {k: sum(1 for s in scenarios if s["status"] == k) for k in statuses}
    out = [WH_INTRO, f"**{len(scenarios)} scenarios**: " + ", ".join(f"{counts[k]} {k}" for k in statuses), ""]
    for area in areas:
        group = [s for s in scenarios if s["area"] == area]
        if not group:
            continue
        out += [f"## {area}", ""]
        for s in group:
            out.append(f"- **{s['id']}** `{s['status']}`: {s['question']}")
            if s.get("note"):
                out.append(f"  - note: {s['note']}")
            if s.get("issues"):
                out.append("  - issues: " + ", ".join(issue_link(lines, n) for n in s["issues"]))
        out.append("")
    by_issue = {}
    for s in scenarios:
        for n in s.get("issues", []):
            by_issue.setdefault(n, []).append(s["id"])
    out += ["## Issue coverage", "", "| issue | status | scenarios |", "|-------|--------|-----------|"]
    for n, r in sorted(table.items()):
        label = issue_link(lines, n) + (f" (see {r['anchor']})" if r["anchor"] != n else "")
        out.append(f"| {label} | {r['status']} | {', '.join(by_issue.get(n, [])) or 'none'} |")
    out.append("")
    return "\n".join(out)


def write_wh_md(dry=False):
    cfg = config()
    lines = read_lines(issue_md())
    text = render_wh(load_wh(), lines, cfg)
    if dry:
        return
    wh_md().write_bytes(text.encode("utf-8"))


def toml_str(v):
    return json.dumps(v, ensure_ascii=False)


def validate_wh_file(cfg):
    errs = check_wh(load_wh(), read_lines(issue_md()), cfg)
    if errs:
        raise Fail("\n".join(errs))


def cmd_wh_add(a):
    cfg = config()
    form = {"area": a.area or "", "issues": ",".join(a.issue), "note": a.note or ""}
    question = a.question or ""
    draft = None
    if not (a.question and a.area):
        tpl = WH_TEMPLATE.format(area=form["area"], issues=form["issues"], note=form["note"], question=question)
        draft = new_draft("wh", tpl)
        open_editor(draft)
        text = draft.read_text(encoding="utf-8")
        if text.strip() == tpl.strip():
            draft.unlink()
            print("nothing to add (draft unchanged)")
            return 0
        form, question = parse_form(text)
    issues = [n.strip() for n in form.get("issues", "").split(",") if n.strip()]
    scenarios = load_wh()
    nums = [int(s["id"].split("-")[1]) for s in scenarios] or [0]
    sid = f"WH-{max(nums) + 1:03d}"
    question = " ".join(question.split())
    block = ["", "[[scenario]]", f"id = {toml_str(sid)}", f"area = {toml_str(form.get('area', ''))}", 'status = "untested"']
    if issues:
        block.append("issues = [" + ", ".join(toml_str(i) for i in issues) + "]")
    block.append(f"question = {toml_str(question)}")
    if form.get("note"):
        block.append(f"note = {toml_str(form['note'])}")
    old = wh_toml().read_bytes().decode("utf-8") if wh_toml().exists() else ""
    new = old.rstrip("\r\n") + "\n" + "\n".join(block) + "\n"
    try:
        tomllib.loads(new)
        errs = check_wh(tomllib.loads(new).get("scenario", []), read_lines(issue_md()), cfg)
        if errs:
            raise Fail("\n".join(errs))
    except (Fail, tomllib.TOMLDecodeError) as e:
        if draft:
            raise Fail(f"{e}\n  your text is kept at {draft}")
        raise
    if a.dry_run:
        commit(wh_toml(), split_lines(old), split_lines(new), True)
        return 0
    commit(wh_toml(), split_lines(old), split_lines(new), False)
    write_wh_md()
    if draft:
        draft.unlink(missing_ok=True)
    print(f"added {sid}")
    return 0


def cmd_wh_set(a):
    cfg = config()
    old = wh_toml().read_bytes().decode("utf-8")
    parts = re.split(r"(?m)^(?=\[\[scenario\]\])", old)
    hit = False
    for i, p in enumerate(parts):
        if re.search(rf'(?m)^id = "{re.escape(a.id)}"\s*$', p):
            hit = True
            p = re.sub(r"(?m)^status = .*$", lambda _m: f"status = {toml_str(a.status)}", p)
            if a.issue:
                line = "issues = [" + ", ".join(toml_str(n) for n in a.issue) + "]"
                if re.search(r"(?m)^issues = ", p):
                    p = re.sub(r"(?m)^issues = .*$", lambda _m: line, p)
                else:
                    p = re.sub(r"(?m)^(status = .*)$", lambda m: m[1] + "\n" + line, p)
            parts[i] = p
    if not hit:
        raise Fail(f"{a.id} not found")
    new = "".join(parts)
    errs = check_wh(tomllib.loads(new).get("scenario", []), read_lines(issue_md()), cfg)
    if errs:
        raise Fail("\n".join(errs))
    commit(wh_toml(), split_lines(old), split_lines(new), a.dry_run)
    if not a.dry_run:
        write_wh_md()
        print(f"{a.id} -> {a.status}")
    return 0


def cmd_wh_list(a):
    for s in load_wh():
        if (a.status and s["status"] != a.status) or (a.area and s["area"] != a.area):
            continue
        print(f"{s['id']}  {s['status']:<9} {s['area']:<10} {s['question']}")
    return 0


def cmd_wh_render(a):
    cfg = config()
    validate_wh_file(cfg)
    write_wh_md()
    print(f"wrote {wh_md().name}")
    return 0


# ----------------------------------------------------------------------------- cli

def build_parser():
    cfg = config()
    areas, wstat = cfg["wh"]["areas"], cfg["wh"]["statuses"]
    ap = argparse.ArgumentParser(prog="issues.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def dry(p):
        p.add_argument("--dry-run", action="store_true", help="print a diff, write nothing")

    p = sub.add_parser("report", help="add a new issue")
    p.add_argument("--comment")
    p.add_argument("--body")
    p.add_argument("--status")
    p.add_argument("--weight")
    p.add_argument("--refers", metavar="NNNN")
    p.add_argument("--resume", action="store_true")
    dry(p)
    p.set_defaults(fn=cmd_report)

    p = sub.add_parser("update", help="append an update to an issue section")
    p.add_argument("num")
    p.add_argument("--text")
    p.add_argument("--status")
    dry(p)
    p.set_defaults(fn=cmd_update)

    p = sub.add_parser("status", help="change one table row")
    p.add_argument("num")
    p.add_argument("status")
    p.add_argument("--comment")
    dry(p)
    p.set_defaults(fn=cmd_status)

    p = sub.add_parser("list")
    p.add_argument("--status")
    p.set_defaults(fn=cmd_list)
    sub.add_parser("next").set_defaults(fn=cmd_next)
    sub.add_parser("check").set_defaults(fn=cmd_check)

    wh = sub.add_parser("wh", help="what-happens scenarios").add_subparsers(dest="wcmd", required=True)
    p = wh.add_parser("add")
    p.add_argument("question", nargs="?")
    p.add_argument("--area", choices=areas)
    p.add_argument("--issue", action="append", default=[])
    p.add_argument("--note")
    dry(p)
    p.set_defaults(fn=cmd_wh_add)
    p = wh.add_parser("set")
    p.add_argument("id")
    p.add_argument("status", choices=wstat)
    p.add_argument("--issue", action="append", default=[])
    dry(p)
    p.set_defaults(fn=cmd_wh_set)
    p = wh.add_parser("list")
    p.add_argument("--status", choices=wstat)
    p.add_argument("--area", choices=areas)
    p.set_defaults(fn=cmd_wh_list)
    wh.add_parser("render").set_defaults(fn=cmd_wh_render)
    return ap


def main(argv=None):
    for stream in (sys.stdout, sys.stderr):
        try:
            stream.reconfigure(encoding="utf-8", errors="replace")
        except Exception:
            pass
    try:
        a = build_parser().parse_args(argv)
        return a.fn(a) or 0
    except Fail as e:
        print(f"error: {e}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
