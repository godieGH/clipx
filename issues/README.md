# Issues and "what happens?" scenarios

Developer-facing bug log for ClipX, plus a list of situations worth trying on real devices. Everything here is plain markdown that lives in git, and one small tool (`issues.py`) keeps it tidy.

| I want to... | Go to |
|---|---|
| See all issues | [INDEX.md](INDEX.md) |
| See the things worth testing | [what_happens.md](what_happens.md) |
| Report a bug | [Report an issue](#report-an-issue) |
| Mark something fixed | [Change status](#change-status-add-an-update-edit) |
| Suggest a scenario to test | [What happens? scenarios](#what-happens-scenarios) |
| Know how contributors avoid clashes | [IDs](#ids) and [Working with branches](#working-with-branches) |

## Why this exists

The ClipX repository is private on GitHub. Public GitHub issues are meant for users of the app; this folder is for developers. The two streams are kept apart on purpose: developers are close to the obvious problems and fix them quickly, while users find experience-driven ones that need real use to show up. Every issue here is a candidate for the next milestone.

Fixed issues are never deleted. They are marked `fixed`, so this folder doubles as history.

## Layout

```text
issues/
├── README.md             this guide
├── INDEX.md              GENERATED: table of every issue
├── what_happens.md       GENERATED: every scenario, grouped by area
├── items/                one file per issue          (you write these)
├── scenarios/            one file per scenario       (you write these)
├── issues.py             the tool (Python 3.11+, standard library only)
├── test_issues.py        the tool's tests
├── config.toml           shared settings (config.local.toml is yours and gitignored)
└── .gitattributes        keeps these files on LF line endings
```

Never edit `INDEX.md` or `what_happens.md` by hand: they are rebuilt from the files in `items/` and `scenarios/`, and CI fails if they are out of date.

## Requirements

Python 3.11 or newer, nothing to install. Run everything from the repository root. On Windows use `py -3` if `python` isn't found.

## IDs

Every issue and scenario gets a random 40-character hex id when it is created, the same shape as a git commit id. It never changes, even when the text does.

- **Shown short.** Tables and output show the first 7 characters, like `git log --oneline`.
- **Accepted short.** Wherever a command wants an `ID`, give any 5 or more leading characters of it, for example `issues.py show 3f2a9`. If two ids start the same, the tool says so and you type a few more.
- **In the file name.** Files are named `<first 12 characters>-<slug>.md`, for example `items/3f2a9c1b8e4d-android-crashes-after-pairing.md`. The full id is in the file's front matter.
- **Why not 0001, 0002...?** Sequential numbers collide: two people reporting on two branches both take the next number, and both append to the same spot in one file. Random ids don't collide, and each record is its own file, so two branches almost never touch the same file.
- **Old numbers still work.** Records migrated from the original `issue.md` and `what_happens.toml` keep their old numbers in a `legacy:` field, so `0005` and `WH-004` keep resolving and old commit messages stay meaningful.

## Report an issue

```bash
python issues/issues.py report
```

Your editor opens on a temporary draft, like `git commit`. Fill in the fields, write the observation under the `---` line, save and close. The tool creates `items/<id>-<slug>.md` and regenerates `INDEX.md`. Closing without changes reports nothing.

| Field | Meaning |
|---|---|
| `title` | short name; it also becomes the file name |
| `status` | `report` (open, the default) or `fixed` |
| `weight` | optional: `weak`, `medium` or `strong` |
| `refers` | optional: id of an issue this one points back to (for example one that looked half-fixed) |
| `comment` | optional one-liner shown in the index; changeable later |
| text below `---` | the body, written in your own words |

No editor? Pass everything as flags:

```bash
python issues/issues.py report --title "Tray Quit prints a teardown error" --body "When ..." --weight weak
python issues/issues.py report --title "Recheck the quit fix" --refers 81186 --body "Still prints it on Windows 10."
```

If validation fails, the draft is kept: continue with `python issues/issues.py report --resume`. Anything obvious and cheap to fix should simply be fixed right away instead of reported.

## Change status, add an update, edit

```bash
python issues/issues.py status 3f2a9 fixed --comment "Fixed in abc1234"
python issues/issues.py status 3f2a9 report --weight strong     # --weight none removes it
python issues/issues.py update 3f2a9 --text "Reproduced on Android as well"   # no --text opens the editor
python issues/issues.py update 3f2a9 --text "Fixed by namespacing the keys" --status fixed
python issues/issues.py edit 3f2a9        # open the file in your editor, then regenerate
python issues/issues.py show 3f2a9        # print the path and the text
python issues/issues.py list --status report
```

`status` and `update` only change the lines they own. The rest of the file, including your wording, stays as it was. You can also edit an issue file directly in any editor: body changes need nothing else, but after changing the title, status, weight, comment or links run `python issues/issues.py render`.

## What happens? scenarios

A scenario is a question of the form *"When X happens, what happens?"*. It doesn't answer the question. It lists a situation worth trying, so a tester can reproduce it on their own devices and report anything unexpected, without needing the code or a known-good behavior to compare against.

Lifecycle: `untested` (nobody tried it), then `tested` (tried, behaved as expected) or `issue` (something unexpected happened; at least one issue must be linked).

```bash
python issues/issues.py wh add                       # opens your editor on a draft
python issues/issues.py wh add "When A and B pair at once, what happens?" --area pairing --issue 3f2a9
python issues/issues.py wh set WH-003 tested
python issues/issues.py wh set 7ae8c issue --issue 3f2a9     # unexpected: link the issue you reported
python issues/issues.py wh list --status untested --area transfer
```

Areas: `pairing`, `connect`, `transfer`, `clipboard`, `discovery`, `protocol`, `ui`, `platform` (set in `config.toml`). A scenario's text below the front matter is an optional note: how to reproduce it, or hints. Questions only, no answers.

## Check

```bash
python issues/issues.py check
python -m unittest discover -s issues
```

`check` verifies that:

- every record parses, has a valid id, and its file name starts with the first 12 characters of that id;
- statuses, weights and areas are on the allowed lists, and `refers` and linked issues point at real issues;
- `INDEX.md` and `what_happens.md` match what the generator produces;
- **every relative link** in `issues/*.md`, and every link from the root README into `issues/`, points at a file that exists and, when it has a `#fragment`, at a heading or anchor that exists.

It also lists open issues that have no scenario yet, as information only.

**Links.** An issue is linked by its file (`items/<id>-<slug>.md`), so clicking it opens that issue directly, with no in-page jump to get wrong. Scenarios are linked to an explicit anchor (`what_happens.md#wh-<id>`) that the generator writes next to each one, which also works in viewers that don't scroll to headings. When you write links by hand in a body, run `check` to see that they resolve.

## Working with branches

Each issue and scenario is a separate new file with a random id, so people reporting on different branches don't conflict, and no one has to rebase just to get a free number. The only shared files are the two generated ones, `INDEX.md` and `what_happens.md`. If git reports a conflict in either one, don't merge it by hand:

```bash
git checkout --theirs issues/INDEX.md issues/what_happens.md   # or --ours, it doesn't matter
python issues/issues.py render
git add issues && git rebase --continue                         # or git commit, for a merge
```

## CI

The `issues` job in `.github/workflows/ci.yaml` runs on every pull request and every push to `main`, on Ubuntu and Windows with Python 3.11. It runs the unit tests and `check`, so a hand-edited generated file, a broken link, or an invalid record fails the pull request. It takes seconds and uses no network.

## Choosing your editor

In order: `ISSUES_EDITOR`, then `[editor] command` in config, then `VISUAL` / `EDITOR`, then the first of `code`, `cursor`, `codium`, `subl`, `nano`, `vim`, `vi` (and `notepad` on Windows) found on `PATH`. Graphical editors get their `--wait` flag added so the tool waits until you close the file. To pin one for your machine, create `issues/config.local.toml` (gitignored):

```toml
[editor]
command = "code --wait"
```

On Windows a path with spaces works if quoted: `command = "\"C:\\Program Files\\...\\Code.exe\" --wait"`. Shared defaults (allowed statuses, weights, areas) live in `config.toml`.

## Safety

Every write command takes `--dry-run`, which prints a diff and changes nothing. Before a file is overwritten, a copy is saved in your OS temp folder (`clipx-issues-drafts`), so nothing extra lands in the repository. Drafts that failed validation are kept there too.

Back to the [repository README](../README.md#tracking-issues).
