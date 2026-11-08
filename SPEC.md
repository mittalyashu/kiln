# Script Library: Desktop App Specification

> Audience: an AI coding model (and human developers) implementing this app. Treat this document as the source of truth. Anything under **Assumptions & Open Questions** is a default chosen by the author and should be confirmed or revised during implementation.

---

## 1. Goal

A desktop app to **save, parameterize, run, and sync reusable scripts** (bash/sh/python/node/etc.) so the user never has to hunt for or re-edit a script again.

Core ideas:

1. Scripts are stored as normal files in a **git repository** and synced across machines.
2. Each script has a **working directory** setting. A **global mode** toggle overrides it for every script, and turning it off restores each script's own setting.
3. Scripts declare **parameters** (PR numbers, commit SHA, input path, etc.) in a comment header. The app renders a form, so values change **without editing the script**, and without creating git commits.
4. Every execution is recorded: runs happen in the background, the same script can run many times (even concurrently), and each run keeps its duration, status, parameters, and logs.

### Motivating example

The user has a script that re-runs failed GitHub workflow runs for a list of PRs (`2062 … 2066`). Today they edit the `PRS=(...)` array in the file every time. With this app, they save the script once, type the PR numbers into a multi-line field, and click Run, as many times as they want.

---

## 2. Tech Stack

| Layer | Choice |
|---|---|
| Desktop shell | **Tauri** (Rust backend) |
| Frontend | **React** + TypeScript |
| Git | Shell out to the **system `git`** binary (no libgit2) |
| Local database | **SQLite** (WAL mode) via `rusqlite` or `sqlx` |
| Secrets | OS keychain via the Rust `keyring` crate |
| Code editor (in-app) | CodeMirror 6 or Monaco (implementer's choice) |
| Platforms | **macOS and Linux** first. Windows is a later goal |

---

## 3. Core Concepts

| Concept | Description |
|---|---|
| **Repo** | The git repository holding script files. Either created by the app or an existing one the user connects (local folder or remote URL). |
| **Script** | A file in the repo. Identified by its repo-relative path. Interpreter comes from its shebang. |
| **Folder** | A directory in the repo. The folder tree in the UI mirrors the repo's directories. |
| **Tag** | A label declared in the script header (`@tags`). Used for filtering. |
| **Parameter** | A named input declared in the script header. Typed. Rendered as a form field. |
| **Secret** | A parameter whose value lives in the OS keychain, never in git, presets, or logs. |
| **Preset** | A named, saved set of parameter values for one script (e.g. "sprint-42 PRs"). Local only. |
| **Run** | One execution of a script with a resolved set of values. Has status, timing, and logs. |
| **Schedule** | A cron expression bound to a script + preset. |
| **Working directory (cwd)** | The directory the script process starts in. |
| **Global mode** | App-wide toggle that forces one cwd for all scripts. |

---

## 4. Features

### 4.1 Script library and organization

- Sidebar shows a **folder tree** (mirrors the repo), plus **tag filters** and a **search box**.
- Search matches script name, description, tags, and optionally content.
- Create, rename, move, duplicate, and delete scripts and folders. These operate on real files in the repo.
- Import an existing script by pasting content or choosing a file.
- Rename/move should use `git mv` semantics, so history is preserved.

### 4.2 Editing

- **Built-in editor** with syntax highlighting chosen from shebang/extension.
- **External editor support**: "Open in editor" button. The app **watches the file** for changes and reloads (with a prompt if there are unsaved in-app edits).
- The editor shows a **parameter panel** parsed from the header, so the user can see what the form will look like.

### 4.3 Git sync

- The app uses the system `git`. On startup, verify `git` is installed and show a clear error if not.
- Remote authentication relies on the user's existing setup (SSH agent, credential helper). The app does **not** manage git credentials.
- **Commit policy (important):**
  - Commits happen **only when script files change** (content, rename, move, delete).
  - Running a script, changing parameter values, saving presets, and viewing logs **never** create commits.
  - Auto-commit is **debounced** (default 45 seconds after the last edit) and batches multiple changed files into one commit. A manual **"Save version"** button commits immediately with an optional message.
  - Auto-generated message format: `Update <script>` / `Update N scripts`, with a clear body listing files.
- **Pull:** automatically on app launch, and via a manual **Sync** button: `git pull --rebase --autostash`.
- **Push:** via the **Sync** button (pull + push). Optional setting for auto-push after commit (default off).
- **Conflicts:** if a rebase conflicts, abort/pause sync and show a banner: *"Sync paused: conflict in `<file>`"* with actions **Keep mine**, **Keep theirs**, **Open in external editor**. No in-app merge editor in v1.
- Show sync status in the UI (clean / local changes / ahead / behind / conflict / error).

### 4.4 Working directory and global mode

- Every script can have its own **working directory** (folder picker).
- **Global mode** is an app-level toggle with one **global path**.
  - **ON:** every run uses the global path as cwd, regardless of per-script settings.
  - **OFF:** every run uses that script's own per-script path.
- **Per-script paths must never be overwritten or cleared by global mode.** Toggling global mode off must restore the exact per-script values.
- Effective cwd resolution order: `global mode ON → global path`, else `script's own path`, else `fallback (see below)`.
- Working directories are **machine-specific** and stored locally (never in git).
- **If the cwd doesn't exist at run time:** the run is **not started**; the UI shows a clear error with a button **"Create directory and run"** and a link to change the path.
- If a script has no path set and global mode is off, fall back to the script's own folder within the repo.

### 4.5 Parameters (variable injection)

#### Declaration: comment header

Parameters live in comments at the top of the script, so the file stays a normal, portable script that still runs in a plain terminal. The parser must support both comment styles: `#` (bash, sh, zsh, python, ruby, etc.) and `//` (node, deno, etc.), chosen by shebang/extension.

```bash
#!/usr/bin/env bash
# @name        Re-run failed CI for PRs
# @description Re-runs failed workflow runs on the latest commit of each PR.
# @tags        github, ci
# @param PRS        list     "PR numbers to process"   required
# @param ENV        select   "Target environment"      options="dev,staging,prod" default="dev"
# @param DRY_RUN    boolean  "Print instead of run"    default="false"
# @param RETRIES    number   "Retry count"             default="3"
# @param INPUT_DIR  path     "Input directory"         kind="dir"
# @secret GITHUB_TOKEN "GitHub token"
set -euo pipefail
```

Grammar (informal):

```
@param  NAME  TYPE  "Description"  [required] [default="..."] [options="a,b,c"] [kind="file|dir"]
@secret NAME  "Description"  [required]
@name / @description / @tags   (metadata)
```

#### Supported types (v1)

| Type | UI control | Notes |
|---|---|---|
| `string` | text field | |
| `number` | number field | validate numeric |
| `boolean` | toggle | passed as `true` / `false` |
| `list` | multi-line textarea, one item per line | blank lines ignored; items trimmed |
| `path` | file or folder picker | `kind="file"` or `kind="dir"` |
| `select` | dropdown | fixed `options` |
| secret (`@secret`) | masked field, "stored in keychain" | see 4.6 |

Required parameters block the Run button until filled. Unknown or malformed header lines produce a visible warning in the editor, not a crash.

#### Injection mechanism

The app **never does text find-and-replace inside the script** (quoting and injection risks). Instead, it passes values to the process:

1. **Environment variables (default, all interpreters).** Each parameter becomes an env var named exactly as declared.
2. **List serialization**, because interpreters differ:
   - `NAME`: items joined by **newline** (works in bash 3.2+, sh, zsh, and trivially in any language).
   - `NAME_JSON`: the same list as a **JSON array** (convenient for Python/Node).
   - Bash consumption examples that work even on macOS's bash 3.2 (no `mapfile` needed):
     ```bash
     for pr in $PRS; do ... done                      # word-splitting on newlines
     while IFS= read -r pr; do ... done <<< "$PRS"    # safest for items with spaces
     ```
   - Plain POSIX `sh` has no arrays, so newline-separated text is the supported form there.
3. **Optional generated preamble (bash/zsh only, opt-in per script via `# @inject preamble`).** The app writes a temporary copy of the script with a generated block (e.g. `PRS=(2062 2063 2064)`) inserted **after the shebang**, and runs the temp copy. The saved script is never modified. Note line numbers in error messages shift by the preamble length. Shown in the UI.
4. Booleans/numbers are passed as plain strings; scripts parse them.

Optional later: positional/CLI argument templates (e.g. `--pr {{PRS}}`).

#### Where parameter values live

| Layer | Stored where | In git? |
|---|---|---|
| Parameter *definitions* + defaults | Script header | Yes (it's the script) |
| Named **presets** | Local DB | **No** (optional export later) |
| **Last-used** values | Local DB | **No** |
| **Secrets** | OS keychain | **No** |

### 4.6 Secrets

- Declared with `# @secret NAME "Description"`.
- First use prompts the user; the value is stored in the **OS keychain** (macOS Keychain, Linux Secret Service/libsecret) keyed by app + secret name.
- Injected into the process as an environment variable.
- **Never** written to git, presets, run-history parameter records, or logs.
- Log pipeline **redacts** any occurrence of a secret's value (replace with `***`) before storing or displaying.
- If the keychain is unavailable (e.g. headless Linux without Secret Service), show a clear error. Do not silently fall back to plaintext.

### 4.7 Execution engine

**Environment resolution (important on macOS):** GUI apps do not inherit the terminal's `PATH`, so tools like `pkgx`, `gh`, or Homebrew binaries may not be found. Therefore:

- On app startup, run the user's login shell once (e.g. `$SHELL -l -i -c 'env -0'` with a sentinel to ignore profile noise), parse the environment, and **cache it**.
- Every script run starts from that cached environment, plus parameter and secret env vars.
- Provide a **"Reload environment"** button and an advanced setting to override/extend `PATH`.

**Interpreter selection:**

- Parse the **shebang** (`#!/usr/bin/env bash`, `#!/usr/bin/env python3`, `#!/bin/sh`, `#!/usr/bin/env node`, etc.). Resolve `env X` using the cached `PATH`.
- Spawn the interpreter explicitly with the script path (don't depend on the exec bit).
- No shebang: fall back to bash and show a warning. (Extension-based inference is optional.)

**Run behavior:**

- Runs are **background processes**; the UI never blocks.
- The **same script can run multiple times, including concurrently**. Each is an independent Run.
- Start each process in its **own process group** (`setsid`/`setpgid`) so cancel can kill child processes too.
- **Cancel/kill:** send `SIGTERM` to the process group, wait a grace period (default 5s), then `SIGKILL`. Run status becomes `cancelled`.
- **Live logs:** stream stdout and stderr to the UI via Tauri events while running; also persist to a log file. Preserve stream (stdout vs stderr) and render ANSI colors.
- Log size guard: configurable max size per run (default e.g. 50 MB), truncate with a notice.

**Run record (SQLite):**

| Field | Notes |
|---|---|
| `id` | |
| `script_path` | repo-relative |
| `script_hash` | content hash at run time |
| `params` | non-secret values; secret *names* only |
| `cwd` | effective working directory used |
| `interpreter` | resolved command |
| `trigger` | `manual` or `schedule` |
| `started_at`, `ended_at`, `duration_ms` | duration shown in UI |
| `exit_code` | |
| `status` | `running` / `succeeded` / `failed` / `cancelled` |
| `log_path` | log file on disk |

**Run history UI:**

- Per script: list of runs with status, start time, **duration**, exit code, trigger, and parameter summary.
- Click a run to view its full **logs** (with search and copy).
- "Re-run with same values" action.
- Retention: **configurable in settings**; default keep the **last 50 runs per script**. Old logs are deleted with their records.

**Notifications:** desktop notification when a run finishes (script name, status, duration). Global setting with per-script override.

### 4.8 Change review and strict mode

Because scripts can change via `git pull` from another machine:

- The app stores, **per machine**, the content hash each script had when last **reviewed/run here**.
- After a pull, any script whose current hash differs shows a **warning banner** on that script: *"This script changed since you last reviewed it"*, with a **diff view** and a **Mark as reviewed** button.
- **Strict mode** (global setting, **off by default**): when ON, changed-and-unreviewed scripts **cannot be run** (manual or scheduled) until reviewed. Scheduled runs that are blocked are recorded as `blocked` and trigger a notification.
- The check lives inside the run function (not only the Run button) so no entry point can bypass it.

### 4.9 Scheduling

- A **Schedule** = script + preset + cron expression + enabled flag.
- **v1:** schedules fire **only while the app is running**. A **tray/menu-bar icon** keeps the app alive when the window is closed.
- Scheduled runs use the bound preset's values (there is nobody to fill the form). Required params/secrets missing → run recorded as `failed` with a clear reason.
- Missed runs (app was closed at fire time): **skip**, no catch-up (default).
- UI to create/edit/disable schedules and see the next run time.

---

## 5. Data Storage Summary

| Data | Location | Synced via git? |
|---|---|---|
| Script files | Repo | Yes |
| Folder structure | Repo directories | Yes |
| Name, description, tags, param definitions | Script header | Yes |
| Per-script working directory | Local DB | No |
| Global mode toggle + global path | Local DB | No |
| Presets, last-used values | Local DB | No |
| Schedules | Local DB | No |
| Reviewed hashes | Local DB | No |
| Run history | Local DB | No |
| Logs | Local files (app data dir) | No |
| Secrets | OS keychain | No |
| App settings | Local DB / config file | No |

Local DB rows referencing scripts are keyed by **repo id + repo-relative path**, and must be updated on rename/move.

---

## 6. Architecture Guidelines

Designed so scheduling can later move out-of-process (see §8) without a rewrite.

1. **One Rust module owns all run logic** (resolve cwd, resolve env, inject params/secrets, check strict mode, spawn, stream logs, record history, notify). The UI and the scheduler call the **same** `run_script(script, values, trigger)` function. **No run logic in React.**
2. **Runs are preset-shaped:** a run is `script + resolved values`. Manual runs, re-runs, and schedules all produce the same input.
3. **The database is independent of the window:** own migrations, WAL mode, safe for a second process to open.
4. **Strict-mode/change checks are inside `run_script`**, not in button handlers.
5. Suggested Rust modules: `git`, `scripts` (scan/parse headers), `params`, `env` (login-shell capture), `runner`, `secrets`, `scheduler`, `db`, `notify`, `commands` (Tauri IPC layer).
6. Frontend ↔ backend: Tauri commands for actions; Tauri events for live log lines and run status changes.
7. Header parser is a standalone, well-tested unit (both comment styles, malformed input, quoting).

---

## 7. Platform Support

- **v1:** macOS and Linux.
- Avoid hard-coding POSIX assumptions in places that would block a future Windows port (use path-handling crates; isolate process-group logic behind an abstraction).
- Windows (later): would need Git Bash/WSL for shell scripts and a Windows implementation of process-tree kill, keychain, and env capture.

---

## 8. Long-Term Goals (not v1)

1. **OS-level scheduling:** register launchd (macOS) / systemd timers (Linux) that launch the app binary in a **headless mode** (e.g. `app run --script X --preset Y`), so schedules fire when the app is closed. Requires: a headless entry point, concurrent SQLite access from two processes, login-shell env capture in headless mode, keychain access outside the GUI, and the same strict-mode/notification behavior. Architecture rules in §6 exist to make this a modest change.
2. Windows support.
3. A CLI to run saved scripts from the terminal.
4. Preset export/import (shareable via git as an opt-in).
5. In-app merge/conflict editor.
6. Positional/CLI argument templates in addition to env vars.
7. Run chaining/pipelines.

---

## 9. Assumptions & Open Questions

These were either not explicitly answered or were the author's recommendation. Confirm or change during implementation.

| # | Topic | Default used in this spec |
|---|---|---|
| 1 | Git auto-push | Off. Push happens on the Sync button. Auto-commit (debounced) is on |
| 2 | Auto-commit debounce | 45 seconds |
| 3 | Missing cwd at run time | Block the run, offer "Create directory and run" |
| 4 | Metadata split (portable vs local) | Portable in script header/git; machine-specific local only |
| 5 | List serialization | Newline-separated env var + `NAME_JSON`; optional bash preamble |
| 6 | Repo setup | Support both new repo and connecting an existing one |
| 7 | Conflict handling | Pause sync, keep mine/theirs/external editor |
| 8 | Run history retention | Configurable; default last 50 per script |
| 9 | Log size cap | 50 MB per run |
| 10 | Cancel grace period | 5 seconds, then SIGKILL |
| 11 | Missed scheduled runs | Skipped |
| 12 | Scripts without a shebang | Run with bash and warn |
| 13 | Strict mode default | Off |

---

## 10. Suggested Milestones

1. **Foundation:** Tauri + React shell, repo setup/connect, script tree, search, built-in + external editing, git commit/pull/push with conflict banner.
2. **Parameters + runner:** header parser, parameter form, login-shell env capture, background runs, live logs, cancel, run history with duration, presets.
3. **Paths, secrets, safety:** per-script working dir + global mode, keychain secrets with log redaction, change-review banner + strict mode.
4. **Scheduling and polish:** cron schedules, tray icon, desktop notifications, retention settings, error handling pass.

---

## 11. Acceptance Criteria (v1 highlights)

- Saving a script and running it 10 times with different PR lists produces **zero** git commits beyond the initial one.
- Toggling global mode ON then OFF returns every script to its original per-script working directory.
- A script calling `gh` or `pkgx` runs successfully when launched from the Dock/Finder, matching terminal behavior.
- Two simultaneous runs of the same script have separate logs, statuses, and durations.
- Cancelling a run kills its child processes, not just the top-level process.
- A secret value never appears in git, the database, or any displayed/stored log.
- After a pull that changes a script, that script shows the review banner; with strict mode on it cannot run until marked reviewed.
