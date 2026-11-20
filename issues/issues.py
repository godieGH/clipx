#!/usr/bin/env python3
"""issues: the tool behind issues/   (python >= 3.11, stdlib only)

Every issue is one file in issues/items/, every "what happens?" scenario is one file in
issues/scenarios/. Each has a random 40-hex id (like a git commit id): shown shortened,
accepted shortened. INDEX.md and what_happens.md are generated from those files.

  issues.py report                  open your editor on a temp draft, then create the issue file
  issues.py report --title "..." --body "..." [--comment "..."] [--weight weak] [--refers ID]
  issues.py report --resume         reopen the last draft that failed validation
  issues.py update ID [--text "..."] [--status fixed]    append an **Update:** block
  issues.py status ID fixed [--comment "..."] [--weight weak|medium|strong|none]
  issues.py edit ID                 open the file (issue or scenario) in your editor
  issues.py show ID                 print the path and the text
  issues.py list [--status S]       print the issues

  issues.py wh add ["question?"] [--area A] [--issue ID] [--note T]
  issues.py wh set ID tested|untested|issue [--issue ID]
  issues.py wh list [--status S] [--area A]

  issues.py render                  regenerate INDEX.md and what_happens.md
  issues.py check                   validate files, generated output and every link

ID = at least 5 hex characters of the id, or a legacy number (0004, WH-004) of a migrated record.
Write commands take --dry-run (print a diff, write nothing). Full guide: issues/README.md
Config: issues/config.toml (shared) and issues/config.local.toml (yours, gitignored).
"""
import argparse
import difflib
import hashlib
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
from urllib.parse import unquote

DIR = Path(os.environ.get("ISSUES_DIR") or Path(__file__).resolve().parent)
FILE_LEN, SHOW_LEN, MIN_PREFIX = 12, 7, 5
ID_RE = re.compile(r"[0-9a-f]{40}")
TS_RE = re.compile(r"\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ")
ORDER = {
    "issue": ["id", "legacy", "title", "status", "weight", "refers", "created", "comment"],
    "scenario": ["id", "legacy", "area", "status", "issues", "created", "question"],
}
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


# ----------------------------------------------------------------------------- paths, config, io

def items_dir():
    return DIR / "items"


def scen_dir():
    return DIR / "scenarios"


def index_md():
    return DIR / "INDEX.md"


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
            for sec, vals in tomllib.loads(read(p)).items():
                cfg.setdefault(sec, {}).update(vals)
    return cfg


def read(path):
    return path.read_bytes().decode("utf-8")


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(path.name + ".tmp")
    tmp.write_bytes(text.encode("utf-8"))
    os.replace(tmp, path)


def commit(path, old, new, dry):
    """Write `new` to path (a backup of the old file goes to the OS temp folder), or print a diff."""
    if dry:
        sys.stdout.writelines(difflib.unified_diff(old.splitlines(True), new.splitlines(True),
                                                   f"a/{path.name}", f"b/{path.name}"))
        print("(dry run: nothing written)")
        return
    if path.exists():
        shutil.copy2(path, drafts_dir() / f"{path.name}.{time.strftime('%Y%m%d-%H%M%S')}.bak")
    write(path, new)


def now_utc():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


# ----------------------------------------------------------------------------- record files

FM = re.compile(r"\A---\n(.*?)\n---[ \t]*(?:\n|\Z)(.*)\Z", re.S)


def parse_record(text):
    m = FM.match(text.replace("\r\n", "\n"))
    if not m:
        raise Fail("front matter not found (the file must start with a '---' block)")
    meta = {}
    for line in m[1].split("\n"):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        k, sep, v = line.partition(":")
        if not sep:
            raise Fail(f"bad front matter line: {line!r}")
        meta[k.strip().lower()] = v.strip()
    return meta, m[2].strip("\n")


def dump_record(kind, meta, body):
    order = ORDER[kind]
    keys = [k for k in order if meta.get(k)] + sorted(k for k in meta if k not in order and meta[k])
    head = "".join(f"{k}: {meta[k]}\n" for k in keys)
    return f"---\n{head}---\n\n{body.strip()}\n" if body.strip() else f"---\n{head}---\n"


def set_fields(text, updates):
    """Edit front matter lines only. value None = leave, "" = remove the line. The body is untouched."""
    lines = text.splitlines(keepends=True)
    if not lines or lines[0].strip() != "---":
        raise Fail("front matter not found")
    end = next((i for i in range(1, len(lines)) if lines[i].strip() == "---"), None)
    if end is None:
        raise Fail("front matter is not closed with '---'")
    eol = "\r\n" if lines[0].endswith("\r\n") else "\n"
    for key, val in updates.items():
        if val is None:
            continue
        pat = re.compile(r"^" + re.escape(key) + r"\s*:", re.I)
        idx = next((i for i in range(1, end) if pat.match(lines[i])), None)
        if val == "":
            if idx is not None:
                del lines[idx]
                end -= 1
        elif idx is not None:
            lines[idx] = f"{key}: {val}{eol}"
        else:
            lines.insert(end, f"{key}: {val}{eol}")
            end += 1
    return "".join(lines)


def append_update(text, new_text, cfg):
    if not new_text.strip():
        raise Fail("update text is empty")
    eol = "\r\n" if "\r\n" in text else "\n"
    label = f"Update ({time.strftime('%Y-%m-%d')})" if cfg["report"]["update_dates"] else "Update"
    parts = new_text.replace("\r\n", "\n").strip().split("\n")
    parts[0] = f"**{label}:** " + parts[0]
    return text.rstrip("\r\n") + eol + eol + eol.join(parts) + eol


def order_key(r):
    m = r["meta"]
    if m.get("legacy"):
        return (0, m["legacy"].lower(), "")
    return (1, m.get("created", ""), m.get("id", ""))


def load_all(kind, errs=None):
    d = items_dir() if kind == "issue" else scen_dir()
    out = []
    for p in (sorted(d.glob("*.md")) if d.is_dir() else []):
        try:
            meta, body = parse_record(read(p))
        except Fail as e:
            if errs is None:
                raise Fail(f"{d.name}/{p.name}: {e}")
            errs.append(f"{d.name}/{p.name}: {e}")
            continue
        out.append({"path": p, "meta": meta, "body": body, "kind": kind})
    return sorted(out, key=order_key)


def rel(r):
    return f"{r['path'].parent.name}/{r['path'].name}"


def refs(r):
    return [x for x in re.split(r"[,\s]+", r["meta"].get("issues", "")) if x]


def short(r, pool):
    i = r["meta"]["id"]
    n = SHOW_LEN
    while n < len(i) and sum(1 for o in pool if o["meta"].get("id", "").startswith(i[:n])) > 1:
        n += 1
    return i[:n]


def scen_label(s, scens):
    return s["meta"].get("legacy") or short(s, scens)


def resolve(ref, pool, kind=None):
    """A legacy number (0004, WH-004) or at least MIN_PREFIX hex characters of the id."""
    ref = ref.strip().lower()
    cand = [r for r in pool if kind in (None, r["kind"])]
    hit = [r for r in cand if r["meta"].get("legacy", "").lower() == ref]
    if not hit and re.fullmatch(r"[0-9a-f]{%d,40}" % MIN_PREFIX, ref):
        hit = [r for r in cand if r["meta"].get("id", "").startswith(ref)]
    what = kind or "record"
    if not hit:
        raise Fail(f"no {what} matches {ref!r} (use {MIN_PREFIX}+ hex characters of its id, or its legacy number)")
    if len(hit) > 1:
        raise Fail(f"{ref!r} is ambiguous, it matches {', '.join(x['meta']['id'][:SHOW_LEN + 2] for x in hit)}")
    return hit[0]


def new_id(seed, taken):
    while True:
        h = hashlib.sha1(f"{seed}\n{time.time_ns()}\n{os.urandom(16).hex()}".encode("utf-8")).hexdigest()
        if not any(t[:FILE_LEN] == h[:FILE_LEN] for t in taken):
            return h


def slugify(text, fallback):
    s = re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")
    if len(s) > 48:
        s = s[:48].rsplit("-", 1)[0] if "-" in s[:48] else s[:48]
    return s.strip("-") or fallback


def file_name(rid, text, fallback):
    return f"{rid[:FILE_LEN]}-{slugify(text, fallback)}.md"


# ----------------------------------------------------------------------------- validation

def validate_records(cfg, issues, scens):
    errs = []
    seen = {}
    for r in issues + scens:
        m, name = r["meta"], rel(r)
        rid = m.get("id", "")
        if not ID_RE.fullmatch(rid):
            errs.append(f"{name}: id must be 40 lowercase hex characters")
            continue
        if not r["path"].name.startswith(rid[:FILE_LEN] + "-"):
            errs.append(f"{name}: the file name must start with the first {FILE_LEN} characters of its id ({rid[:FILE_LEN]}-)")
        if rid[:FILE_LEN] in seen:
            errs.append(f"{name}: id starts like {seen[rid[:FILE_LEN]]}; ids must differ within their first {FILE_LEN} characters")
        seen[rid[:FILE_LEN]] = name
        if not m.get("legacy") and not TS_RE.fullmatch(m.get("created", "")):
            errs.append(f"{name}: created must look like 2026-11-19T10:22:31Z")
    legacy = {}
    for r in issues + scens:
        lg = r["meta"].get("legacy", "").lower()
        if lg:
            if (r["kind"], lg) in legacy:
                errs.append(f"{rel(r)}: legacy number {lg!r} is used twice")
            legacy[(r["kind"], lg)] = True
    issue_ids = {r["meta"].get("id") for r in issues}
    for r in issues:
        m, name = r["meta"], rel(r)
        if not m.get("title"):
            errs.append(f"{name}: title is empty")
        if m.get("status") not in cfg["report"]["statuses"]:
            errs.append(f"{name}: status {m.get('status')!r} must be one of {', '.join(cfg['report']['statuses'])}")
        if m.get("weight") and m["weight"] not in cfg["report"]["weights"]:
            errs.append(f"{name}: weight {m['weight']!r} must be one of {', '.join(cfg['report']['weights'])}")
        if m.get("refers") and m["refers"] not in issue_ids:
            errs.append(f"{name}: refers to {m['refers']!r}, which is not an issue id")
        if not m.get("refers") and not r["body"].strip():
            errs.append(f"{name}: the text below the front matter is empty")
    for r in scens:
        m, name = r["meta"], rel(r)
        if not " ".join(m.get("question", "").split()):
            errs.append(f"{name}: question is empty")
        if m.get("area") not in cfg["wh"]["areas"]:
            errs.append(f"{name}: area {m.get('area')!r} must be one of {', '.join(cfg['wh']['areas'])}")
        if m.get("status") not in cfg["wh"]["statuses"]:
            errs.append(f"{name}: status {m.get('status')!r} must be one of {', '.join(cfg['wh']['statuses'])}")
        for i in refs(r):
            if i not in issue_ids:
                errs.append(f"{name}: issues lists {i!r}, which is not an issue id")
        if m.get("status") == "issue" and not refs(r):
            errs.append(f"{name}: status 'issue' needs at least one issue")
    return errs


# ----------------------------------------------------------------------------- generated files

INDEX_INTRO = """\
# Issues

> GENERATED FILE: do not edit by hand. Every issue is a file in `items/`; change one, then run `python issues/issues.py render`. How to use the tool: [README.md](README.md).

Developer-facing bug log, kept apart from the public GitHub issues (see [Why this exists](README.md#why-this-exists)). Fixed issues are never deleted, they are marked `fixed`.
"""

WH_INTRO = """\
# What Happens?

> GENERATED FILE: do not edit by hand. Every scenario is a file in `scenarios/`; add one with `python issues/issues.py wh add`.

> This is a repo way of asking what might happen if a certain behavior, action or event in this software is done or happens. What other expected or unexpected behaviors might be triggered. It does not answer what the triggered behaviors are; it lists the questions for what can trigger them. This lets users or devs reproduce and test on their own side, without a known reference behavior, so they can shape and report unexpected behaviors.

**How to use:** pick an `untested` scenario, try it on your devices, then report what you saw. If something unexpected happened, report it with `python issues/issues.py report` and link it (`wh set ID issue --issue ID`).
"""


def esc(s):
    return " ".join(str(s).split()).replace("|", "\\|")


def issue_link(r, pool):
    return f"[{short(r, pool)}](items/{r['path'].name})"


def render_index(cfg, issues, scens):
    by_id = {r["meta"]["id"]: r for r in issues}
    cover = {}
    for s in scens:
        for i in refs(s):
            cover.setdefault(i, []).append(s)
    counts = ", ".join(f"{sum(1 for r in issues if r['meta']['status'] == k)} {k}" for k in cfg["report"]["statuses"])
    out = [INDEX_INTRO, f"**{len(issues)} issues**: {counts}", "",
           "| id | status | issue | comment | scenarios |", "|----|--------|-------|---------|-----------|"]
    for r in issues:
        m = r["meta"]
        status = m["status"] + (f"({m['weight']})" if m.get("weight") else "")
        title = f"**{esc(m['title'])}**" + (f" (was {m['legacy']})" if m.get("legacy") else "")
        note = esc(m.get("comment", ""))
        if m.get("refers") in by_id:
            note = f"↳ refers to {issue_link(by_id[m['refers']], issues)}. {note}".strip()
        sc = ", ".join(f"[{scen_label(s, scens)}](what_happens.md#wh-{s['meta']['id'][:FILE_LEN]})"
                       for s in cover.get(m["id"], []))
        out.append(f"| {issue_link(r, issues)} | {status} | {title} | {note} | {sc} |")
    return "\n".join(out) + "\n"


def render_wh(cfg, issues, scens):
    areas, statuses = cfg["wh"]["areas"], cfg["wh"]["statuses"]
    by_id = {r["meta"]["id"]: r for r in issues}
    counts = ", ".join(f"{sum(1 for s in scens if s['meta']['status'] == k)} {k}" for k in statuses)
    out = [WH_INTRO, f"**{len(scens)} scenarios**: {counts}", ""]
    for area in areas:
        group = [s for s in scens if s["meta"].get("area") == area]
        if not group:
            continue
        out += [f"## {area}", ""]
        for s in group:
            m = s["meta"]
            anchor = "wh-" + m["id"][:FILE_LEN]
            question = " ".join(m["question"].split())
            out.append(f'- <a id="{anchor}"></a>**{scen_label(s, scens)}** `{m["status"]}`: {question}')
            note = " ".join(s["body"].split())
            if note:
                out.append(f"  - note: {note}")
            links = [issue_link(by_id[i], issues) for i in refs(s) if i in by_id]
            if links:
                out.append("  - issues: " + ", ".join(links))
        out.append("")
    return "\n".join(out)


def generated(cfg, issues, scens):
    return ((index_md(), render_index(cfg, issues, scens)), (wh_md(), render_wh(cfg, issues, scens)))


def render_all(dry=False):
    cfg, issues, scens = config(), load_all("issue"), load_all("scenario")
    errs = validate_records(cfg, issues, scens)
    if errs:
        raise Fail("\n".join(errs))
    for path, text in generated(cfg, issues, scens):
        old = read(path) if path.exists() else ""
        if old.replace("\r\n", "\n") != text:
            commit(path, old, text, dry)


# ----------------------------------------------------------------------------- link checker

LINK = re.compile(r"(?<!!)\[[^\]\n]*\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")


def strip_fences(text):
    return re.sub(r"(?ms)^[ \t]*```.*?^[ \t]*```[^\n]*$", "", text.replace("\r\n", "\n"))


def strip_code(text):
    return re.sub(r"`[^`\n]*`", "", strip_fences(text))


def gh_slug(heading):
    h = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", heading)
    h = re.sub(r"[`*]", "", h).strip().lower()
    return re.sub(r"[^\w\- ]", "", h).replace(" ", "-")


def anchors(path, cache):
    if path not in cache:
        text = strip_fences(read(path))
        seen, out = {}, set()
        for m in re.finditer(r"(?m)^#{1,6}[ \t]+(.+?)[ \t]*#*[ \t]*$", text):
            s = gh_slug(m[1])
            n = seen.get(s, 0)
            seen[s] = n + 1
            out.add(s if n == 0 else f"{s}-{n}")
        out.update(a.lower() for a in re.findall(r'<a\s+(?:id|name)="([^"]+)"', text))
        cache[path] = out
    return cache[path]


def check_links():
    """Every relative link in issues/*.md (and the links into issues/ from the root README) must
    point at a file that exists and, when it has a #fragment, at a heading or anchor that exists."""
    errs, cache, root = [], {}, DIR.parent
    scan = [(p, p.parent, False) for p in sorted(DIR.rglob("*.md"))]
    if (root / "README.md").exists():
        scan.append((root / "README.md", root, True))
    for p, base, only_issues in scan:
        for m in LINK.finditer(strip_code(read(p))):
            target = m[1]
            if re.match(r"[a-z][a-z0-9+.\-]*:", target, re.I):
                continue
            path, _, frag = target.partition("#")
            path = re.sub(r"^\./", "", path)
            if only_issues and not path.startswith("issues/"):
                continue
            file = (base / unquote(path)).resolve() if path else p
            try:
                name = p.resolve().relative_to(root.resolve()).as_posix()
            except ValueError:
                name = p.name
            if not file.exists():
                errs.append(f"{name}: link ({target}) points to a file that does not exist")
            elif frag and file.suffix == ".md" and file.is_file():
                if unquote(frag).lower() not in anchors(file, cache):
                    errs.append(f"{name}: link ({target}) points to #{frag}, but {file.name} has no such heading or anchor")
    return errs


# ----------------------------------------------------------------------------- editor + drafts

def split_command(raw, windows=None):
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
                       "or export EDITOR, or pass the text with flags (--title/--body)")
    exe = shutil.which(argv[0])
    if not exe:
        raise Fail(f"editor {argv[0]!r} not found on PATH")
    argv[0] = exe
    flag = GUI_WAIT.get(Path(exe).stem.lower())
    if flag and flag not in argv:
        argv.append(flag)
    return argv


def open_editor(path):
    r = subprocess.run(editor_argv() + [str(path)])
    if r.returncode != 0:
        raise Fail(f"editor exited with code {r.returncode}; draft kept at {path}")


def new_draft(kind, text):
    p = drafts_dir() / f"{kind}-{time.strftime('%Y%m%d-%H%M%S')}-{os.getpid()}.md"
    p.write_bytes(text.encode("utf-8"))
    return p


def latest_draft(kind):
    ds = sorted(drafts_dir().glob(f"{kind}-*.md"), key=lambda p: p.stat().st_mtime)
    if not ds:
        raise Fail(f"no saved {kind} draft to resume")
    return ds[-1]


COMMENT_LINE = re.compile(r"^\s*<!--.*-->\s*$")


def parse_form(text, header=True):
    lines = [x for x in text.replace("\r\n", "\n").split("\n") if not COMMENT_LINE.match(x)]
    fields = {}
    if header:
        stripped = [x.strip() for x in lines]
        if "---" not in stripped:
            raise Fail("the '---' line between the fields and the text is missing")
        k = stripped.index("---")
        for x in lines[:k]:
            if not x.strip():
                continue
            m = re.match(r"^(\w+)\s*:\s*(.*)$", x)
            if not m:
                raise Fail(f"bad field line: {x!r}")
            fields[m[1].lower()] = m[2].strip()
        lines = lines[k + 1:]
    return fields, "\n".join(lines).strip()


REPORT_TEMPLATE = """\
<!-- New issue. Whole-line HTML comments like this one are dropped. Save and close the editor to submit. -->
<!-- title = a short name (it becomes the file name). comment = optional one line for the index. weight = weak|medium|strong, optional. -->
<!-- refers = id of an issue this one points back to (e.g. a half-fixed one); then the text below may be short. Close without edits to abort. -->
title: {title}
status: {status}
weight: {weight}
refers: {refers}
comment: {comment}
---
{body}
"""

UPDATE_TEMPLATE = """\
<!-- Update for issue {ref}. Whole-line HTML comments are dropped. Save and close to submit. -->
<!-- Changing the status too? Use:  issues.py update {ref} --status fixed -->

"""

WH_TEMPLATE = """\
<!-- New 'what happens' scenario. Whole-line HTML comments are dropped. Save and close to submit. -->
<!-- Write one question below '---', e.g. "When X happens, what happens?". A question only, never an answer. -->
<!-- issues = ids of related issues, comma separated (optional). note = how to reproduce / hints (optional). -->
area: {area}
issues: {issues}
note: {note}
---
{question}
"""


# ----------------------------------------------------------------------------- record builders

def build_issue(cfg, pool, f, body):
    rc = cfg["report"]
    title = " ".join(f.get("title", "").split())
    if not title:
        raise Fail("title is empty (a short name for the issue)")
    status = f.get("status") or rc["default_status"]
    if status not in rc["statuses"]:
        raise Fail(f"status {status!r} must be one of {', '.join(rc['statuses'])}")
    weight = f.get("weight", "")
    if weight and weight not in rc["weights"]:
        raise Fail(f"weight {weight!r} must be one of {', '.join(rc['weights'])}")
    refers = ""
    if f.get("refers"):
        refers = resolve(f["refers"], [r for r in pool if r["kind"] == "issue"], "issue")["meta"]["id"]
    elif not body.strip():
        raise Fail("the text below '---' is empty")
    rid = new_id(f"{title}\n{body}", [r["meta"]["id"] for r in pool])
    meta = {"id": rid, "title": title, "status": status, "weight": weight, "refers": refers,
            "created": now_utc(), "comment": " ".join(f.get("comment", "").split())}
    return {"kind": "issue", "meta": meta, "body": body.strip(),
            "path": items_dir() / file_name(rid, title, "issue")}


def build_scenario(cfg, pool, f, question):
    area = f.get("area", "")
    if area not in cfg["wh"]["areas"]:
        raise Fail(f"area {area!r} must be one of {', '.join(cfg['wh']['areas'])}")
    question = " ".join(question.split())
    if not question:
        raise Fail("the question is empty")
    issue_pool = [r for r in pool if r["kind"] == "issue"]
    ids = [resolve(x, issue_pool, "issue")["meta"]["id"] for x in re.split(r"[,\s]+", f.get("issues", "")) if x]
    rid = new_id(question, [r["meta"]["id"] for r in pool])
    meta = {"id": rid, "area": area, "status": "untested", "issues": ", ".join(ids),
            "created": now_utc(), "question": question}
    return {"kind": "scenario", "meta": meta, "body": f.get("note", "").strip(),
            "path": scen_dir() / file_name(rid, question, "scenario")}


def save_new(rec, dry):
    commit(rec["path"], "", dump_record(rec["kind"], rec["meta"], rec["body"]), dry)


# ----------------------------------------------------------------------------- commands

def cmd_report(a):
    cfg, issues, scens = config(), load_all("issue"), load_all("scenario")
    draft = None
    form = {"title": a.title or "", "status": a.status or "", "weight": a.weight or "",
            "refers": a.refers or "", "comment": a.comment or ""}
    body = a.body or ""
    if a.resume:
        draft = latest_draft("report")
        form, body = parse_form(read(draft))
    elif not (a.title and (a.body or a.refers)):
        tpl = REPORT_TEMPLATE.format(body=body, **{**form, "status": form["status"] or cfg["report"]["default_status"]})
        draft = new_draft("report", tpl)
        open_editor(draft)
        text = read(draft)
        if text.strip() == tpl.strip():
            draft.unlink()
            print("nothing to report (draft unchanged)")
            return 0
        form, body = parse_form(text)
    try:
        rec = build_issue(cfg, issues + scens, form, body)
    except Fail as e:
        if draft:
            raise Fail(f"{e}\n  draft kept at {draft}\n  continue with: issues.py report --resume")
        raise
    save_new(rec, a.dry_run)
    if not a.dry_run:
        if draft:
            draft.unlink(missing_ok=True)
        render_all()
        print(f"reported {rec['meta']['id'][:SHOW_LEN]}  {rec['path'].relative_to(DIR.parent).as_posix()}")
    return 0


def cmd_update(a):
    cfg, issues = config(), load_all("issue")
    r = resolve(a.ref, issues, "issue")
    text, draft = a.text, None
    if text is None:
        tpl = UPDATE_TEMPLATE.format(ref=a.ref)
        draft = new_draft("update", tpl)
        open_editor(draft)
        raw = read(draft)
        if raw.strip() == tpl.strip():
            draft.unlink()
            print("nothing to add (draft unchanged)")
            return 0
        _, text = parse_form(raw, header=False)
    old = read(r["path"])
    try:
        new = append_update(old, text, cfg)
        if a.status:
            if a.status not in cfg["report"]["statuses"]:
                raise Fail(f"status {a.status!r} must be one of {', '.join(cfg['report']['statuses'])}")
            new = set_fields(new, {"status": a.status})
    except Fail as e:
        if draft:
            raise Fail(f"{e}\n  your text is kept at {draft}")
        raise
    commit(r["path"], old, new, a.dry_run)
    if not a.dry_run:
        if draft:
            draft.unlink(missing_ok=True)
        render_all()
        print(f"updated {short(r, issues)}")
    return 0


def cmd_status(a):
    cfg, issues = config(), load_all("issue")
    r = resolve(a.ref, issues, "issue")
    rc = cfg["report"]
    if a.status not in rc["statuses"]:
        raise Fail(f"status {a.status!r} must be one of {', '.join(rc['statuses'])}")
    weight = a.weight
    if weight == "none":
        weight = ""
    elif weight and weight not in rc["weights"]:
        raise Fail(f"weight {weight!r} must be one of {', '.join(rc['weights'])} (or none)")
    comment = None if a.comment is None else " ".join(a.comment.split())
    old = read(r["path"])
    commit(r["path"], old, set_fields(old, {"status": a.status, "weight": weight, "comment": comment}), a.dry_run)
    if not a.dry_run:
        render_all()
        print(f"{short(r, issues)} -> {a.status}")
    return 0


def cmd_edit(a):
    r = resolve(a.ref, load_all("issue") + load_all("scenario"))
    open_editor(r["path"])
    render_all()
    print(f"edited {rel(r)}")
    return 0


def cmd_show(a):
    r = resolve(a.ref, load_all("issue") + load_all("scenario"))
    print(r["path"])
    print()
    print(read(r["path"]).rstrip("\n"))
    return 0


def cmd_list(a):
    issues = load_all("issue")
    for r in issues:
        m = r["meta"]
        if a.status and m.get("status") != a.status:
            continue
        print(f"{short(r, issues)}  {m.get('status', ''):<7} {m.get('legacy', ''):<5} {m.get('title', '')}")
    return 0


def cmd_render(a):
    render_all(a.dry_run)
    if not a.dry_run:
        print("generated INDEX.md and what_happens.md are up to date")
    return 0


def cmd_check(a):
    cfg, errs = config(), []
    issues, scens = load_all("issue", errs), load_all("scenario", errs)
    errs += validate_records(cfg, issues, scens)
    if not errs:
        for path, text in generated(cfg, issues, scens):
            have = read(path).replace("\r\n", "\n") if path.exists() else None
            if have != text:
                errs.append(f"{path.name} is out of date: run `python issues/issues.py render`")
    errs += check_links()
    if errs:
        print("\n".join(errs), file=sys.stderr)
        return 1
    print(f"ok: {len(issues)} issues, {len(scens)} scenarios, all links resolve")
    covered = {i for s in scens for i in refs(s)}
    todo = [short(r, issues) for r in issues if r["meta"]["status"] == "report" and r["meta"]["id"] not in covered]
    if todo:
        print("open issues with no what-happens scenario: " + ", ".join(todo))
    return 0


def cmd_wh_add(a):
    cfg, issues, scens = config(), load_all("issue"), load_all("scenario")
    form = {"area": a.area or "", "issues": ",".join(a.issue), "note": a.note or ""}
    question, draft = a.question or "", None
    if not (a.question and a.area):
        tpl = WH_TEMPLATE.format(question=question, **form)
        draft = new_draft("wh", tpl)
        open_editor(draft)
        text = read(draft)
        if text.strip() == tpl.strip():
            draft.unlink()
            print("nothing to add (draft unchanged)")
            return 0
        form, question = parse_form(text)
    try:
        rec = build_scenario(cfg, issues + scens, form, question)
    except Fail as e:
        if draft:
            raise Fail(f"{e}\n  your text is kept at {draft}")
        raise
    save_new(rec, a.dry_run)
    if not a.dry_run:
        if draft:
            draft.unlink(missing_ok=True)
        render_all()
        print(f"added {rec['meta']['id'][:SHOW_LEN]}  {rec['path'].relative_to(DIR.parent).as_posix()}")
    return 0


def cmd_wh_set(a):
    issues, scens = load_all("issue"), load_all("scenario")
    r = resolve(a.ref, scens, "scenario")
    ids = [resolve(x, issues, "issue")["meta"]["id"] for x in a.issue]
    if a.status == "issue" and not (ids or refs(r)):
        raise Fail("status 'issue' needs an issue: add --issue ID")
    upd = {"status": a.status}
    if ids:
        upd["issues"] = ", ".join(ids)
    old = read(r["path"])
    commit(r["path"], old, set_fields(old, upd), a.dry_run)
    if not a.dry_run:
        render_all()
        print(f"{scen_label(r, scens)} -> {a.status}")
    return 0


def cmd_wh_list(a):
    scens = load_all("scenario")
    for s in scens:
        m = s["meta"]
        if (a.status and m.get("status") != a.status) or (a.area and m.get("area") != a.area):
            continue
        print(f"{scen_label(s, scens):<8} {m.get('status', ''):<9} {m.get('area', ''):<10} {m.get('question', '')}")
    return 0


# ----------------------------------------------------------------------------- cli

def build_parser():
    cfg = config()
    areas, wstat, weights = cfg["wh"]["areas"], cfg["wh"]["statuses"], cfg["report"]["weights"]
    ap = argparse.ArgumentParser(prog="issues.py", description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def dry(p):
        p.add_argument("--dry-run", action="store_true", help="print a diff, write nothing")

    p = sub.add_parser("report", help="add a new issue")
    for opt in ("--title", "--body", "--comment", "--status", "--weight", "--refers"):
        p.add_argument(opt)
    p.add_argument("--resume", action="store_true")
    dry(p)
    p.set_defaults(fn=cmd_report)

    p = sub.add_parser("update", help="append an update to an issue")
    p.add_argument("ref")
    p.add_argument("--text")
    p.add_argument("--status")
    dry(p)
    p.set_defaults(fn=cmd_update)

    p = sub.add_parser("status", help="change status / weight / comment of an issue")
    p.add_argument("ref")
    p.add_argument("status")
    p.add_argument("--comment")
    p.add_argument("--weight", help=f"{', '.join(weights)} or none")
    dry(p)
    p.set_defaults(fn=cmd_status)

    for name, fn, help_ in (("edit", cmd_edit, "open a record in your editor"), ("show", cmd_show, "print a record")):
        p = sub.add_parser(name, help=help_)
        p.add_argument("ref")
        p.set_defaults(fn=fn)

    p = sub.add_parser("list")
    p.add_argument("--status")
    p.set_defaults(fn=cmd_list)
    p = sub.add_parser("render", help="regenerate INDEX.md and what_happens.md")
    dry(p)
    p.set_defaults(fn=cmd_render)
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
    p.add_argument("ref")
    p.add_argument("status", choices=wstat)
    p.add_argument("--issue", action="append", default=[])
    dry(p)
    p.set_defaults(fn=cmd_wh_set)
    p = wh.add_parser("list")
    p.add_argument("--status", choices=wstat)
    p.add_argument("--area", choices=areas)
    p.set_defaults(fn=cmd_wh_list)
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
