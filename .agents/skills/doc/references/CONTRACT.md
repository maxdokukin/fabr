# The format contract

The agent backend (`crates/fabr-agent`), the core (`crates/fabr-core`) and the app talk through files. The one exception is the setup checks, which run as a local command with JSON on stdout. This file says which files exist, who writes each field, and the protocols on top. Schema 7, as of the Claude Code engine (2026-10-30). The code is the source of truth. Where this file and the code differ, the code wins, and this file should be fixed: field truth is `crates/fabr-core/src/taskrec.rs`, registry truth is `crates/fabr-agent/src/board.rs`. The ten fixtures in `fixtures/` are the contract in executable form, and a change to the record changes them in the same commit.

## Paths

| Path | Format | Written by | Read by |
|---|---|---|---|
| `~/.fabric-space/settings.toml` | TOML | setup (app) | backend, app |
| `~/.fabric-space/secrets.toml` | TOML, mode 600 | setup (app) | backend only |
| `~/.fabric-space/registry/<task_id>--<path_tag>.yaml` | YAML | backend | app |
| `~/.fabric-space/registry/_backend.yaml` | YAML heartbeat | backend | app |
| `~/.fabric-space/events.log` | one line per event | backend | app |
| `~/.fabric-space/telegram.yaml` | YAML | backend (`telegram.rs`) | backend |
| `~/.fabric-space/registry/_runs.txt` | `pgid owner_pid` per line | backend, per Claude Code run | backend on start |
| `<folder>/.fabric-space/chat/<session>.lock` | pid | app chat (`ccsession::send`) | backend before a resume |
| `<folder>/.fabric-space/tasks/<task_id>.md` | YAML front matter plus markdown | both, by field | both |
| `<folder>/.fabric-space/tasks.lock` | flock target | any record writer | any record writer |
| `<folder>/.fabr/meta.toml` | TOML | app, CLI | app, CLI |
| `<folder>/.fabric-space/.agents/rules.md`, `knowledge.md` | markdown | app (node settings) | gatherer |
| `<folder>/.fabric-space/.agents/learned.yaml` | YAML `records:` | learner; app may delete a record | gatherer, app |
| `~/.claude/projects/<folder with every non-alphanumeric as ->/<session>.jsonl` | Claude Code's own store | Claude Code | app (`ccsession.rs`), read-only |

`.fabr/` holds only `meta.toml` (node settings). A `.todo` file in it is a leftover that `fabr import` converts. Remote ssh nodes list their tasks read-only through the snapshot. Planned and not in code: `data-bank/` (vault, policy, audit log), `proposals.yaml`, `index.sqlite`.

Human-edited files are TOML or markdown and machine-written files are YAML. Timestamps are ISO 8601 UTC with a `Z`. A task id is `YYYY-MM-DD-<slug>`, and the record file is `<task_id>.md`. A task is identified by its full path. `path_tag` is the first 8 hex characters of the sha256 of the absolute record path.

## The task record

YAML front matter, then five sections in this order: `## Context`, `## Spec`, `## Success`, `## Plan`, `## Log`. Schemas 2 to 6 still parse and are rewritten as 7.

| Field | Writer | Meaning |
|---|---|---|
| `schema` | creator | `7` |
| `id`, `title` | creator, user | one line each |
| `due` | user | none, a date `YYYY-MM-DD`, or `YYYY-MM-DDTHH:MM` |
| `pri` | user | 0 to 3 |
| `state` | backend (see below) | `reminder`, `task`, `defined`, `in_progress`, `succeeded`, `failed` |
| `step` | backend | `gather`, `define`, `plan`, `approve`, `execute`, `evaluate`, `learn`, `notify`, or null |
| `waiting` | backend | true while a question has no answer or approval is pending |
| `paused` | backend | the agent stops after the current step |
| `suspended`, `suspend_until` | app | the task is shelved until the time passes; the runner clears both |
| `source` | legacy | still parsed, nothing writes it |
| `created_at`, `updated_at` | every writer updates `updated_at` | |
| `requests` | app sets true, backend clears | `delegate`, `cancel`, `redelegate`, `pause`, `resume` |
| `question` | backend creates, app fills `answer` | see below |
| `answers` | backend | answered questions, copied from the slot |
| `grants` | backend `requested`, app `granted` | `granted` is a list of `{field, scope}`, scope `task`, `project` or `always` |
| `plan_meta` | backend | `reversibility`, `tools`, `assumptions`, `checks` (list of `{cmd, args}`) |
| `assertions` | backend | `{id, text}`, the checkable form of the success definition |
| `approval` | backend creates and sets `digest`; app sets `status`, `by`, `at`, `note` | status `pending`, `approved`, `refused`; `by` is `gui`, `telegram`, `cli` or `auto` |
| `verdict` | backend | `result` pass or fail, `at`, `items` of `{id, pass, evidence}` |
| `cost_usd` | backend | model spend for this record |
| `links` | backend | `redelegated_from`, `redelegated_to` |
| `sessions` | backend | `plan`, `execute`, `evaluate`: Claude Code session ids, null until that role has run. `claude --resume <id>` in the node folder reopens one |

`## Context` holds the user's text, and the gatherer appends a `### Gathered` subsection. `## Spec` and `## Plan` are written by the planner session; the plan has `### Inputs`, `### Outputs` and `### Steps`. `## Success` is the user's definition as given. `## Log` is append-only lines `- <timestamp> <step>: <message>` written by the backend; the app never writes it.

The approval digest is a sha256 over the normalized Spec, Success and Plan bodies, the assertions, and `plan_meta` (reversibility, tools, checks). The recipe is `approval_digest` in `taskrec.rs`. It is not the JSON preimage that the first version of this file described.

## States and who moves them

Only the backend writes `state`, with one exception: the app may move a reminder to succeeded by hand, logged as "marked done by hand". Waiting, paused and suspended are flags, not states.

| From | To | Trigger |
|---|---|---|
| reminder | task | `requests.delegate`; the record gets `step: gather` |
| task | defined | the define and plan steps are done and approval is pending |
| defined | in_progress | approval approved, digest matches, not `auto` for an irreversible plan, no other in-progress task in the node |
| defined | defined | approval refused; the plan is redone with the note |
| in_progress | succeeded or failed | verdict pass or fail |
| in_progress | failed | `requests.cancel`, or the third consecutive error on one stage |
| failed | task | `requests.redelegate` creates a new record linked both ways |
| reminder | succeeded | by hand in the app |

A request that does not apply to the current state is cleared and logged as `ignored request <key> in state <state>`. Pause and resume apply to a record with a `step`: pause sets `paused`, resume clears it.

## Questions, approval and concurrent writes

The backend writes the slot and sets `waiting: true`:

```yaml
question:
  id: q3
  kind: open            # open | choice | grant
  text: Which shop?
  options: null         # choice: [once, always, no]
  prior: null           # choice: {text, source_task, last_confirmed}
  fields: null          # grant: [address.line_1, address.zip]
  asked_at: 2026-10-24T18:03:40Z
  answer: null          # app fills {value, replacement, granted, by, at}
```

The app fills `answer` only if it is still null when read. A second writer that finds it set shows the answer that landed. Any backend rewrite of Spec, Success, Plan, assertions or `plan_meta` resets `approval` to pending with a new digest. The app may edit `grants.granted` before approving.

A record writer holds an exclusive flock on `tasks.lock` for the read, the `updated_at` check, a temp file in the same folder, and the rename. If `updated_at` changed, it re-reads and retries once, then stops and logs.

## The registry

One file per task from gather through notify, removed when the task is terminal and the tail steps are done, or on cancel. Keys written by `board.rs`:

`task_path`, `task_id`, `folder`, `state`, `step`, `step_at`, `started_at`, `heartbeat_at`, `heartbeat_interval_s`, `waiting` (`question`, `approval` or null), `activity` (`model`, `tool`, `check`, `parked`, `retry`, `queued`, `paused`, or null), `retry_in_s`, `last_error`, `cost_usd`, `usage` (per step: calls, tokens, usd), `log_tail` (last 20 log lines), `question`, `pid`.

An entry is dead when `heartbeat_at` is older than three intervals. The app marks it, and the runner emits a `stuck` event and removes stale files on start. Suspended tasks drop their entry. `_backend.yaml` holds `heartbeat_at`, `heartbeat_interval_s`, `pid`, `started_at` and `last_error`; the app shows "not connected" when it is absent or stale.

## Events

`events.log` is append-only: `<timestamp> <kind padded to 7> <task path> <detail>`, newlines in the detail replaced by spaces. Kinds: `waiting`, `done`, `failed`, `stuck`. Open issue: the task path is not quoted, so a path with a space breaks the parse. The app keeps its own read position.

## Settings, secrets and checks

`settings.toml` keys the backend reads: `default_folder`, `claude_bin` (empty means `claude` on PATH; env `FABR_CLAUDE_BIN` wins), `model` (empty for Claude Code's default, `sonnet`, `opus`, `haiku`, `fable`, or a full `claude-` name), `effort` (empty or `low` to `max`), `max_budget_usd` (0 for no cap; passed as `--max-budget-usd`), `max_task_usd` (0 for no cap on the task's total), `execute_timeout_s` (default 1800, the wall-clock limit of one Claude Code run), `heartbeat_interval_s` (default 10), `check_timeout_s` (default 60, at most 3600), `auto_mode`, `execute` and `[setup] completed_steps`. The backend starts when `completed_steps` has `folder` and `claude` and the binary resolves. `secrets.toml` holds `telegram_bot_token`, mode 600, never loaded into an agent context; an old `anthropic_api_key` line is left alone and unused. The bot token is redacted from every log line, event and record.

The setup wizard has five screens: default folder, Claude Code, Telegram bot, Telegram link, done. The two Telegram screens can be skipped. The checks are `fabr check folder <path>`, `fabr check claude`, `fabr check telegram-token` and `fabr telegram link`. Each prints one JSON object with `ok`, and on failure a `reason` the user can act on, and exits 1. `check claude` runs `claude --version` and `claude auth status` and never sends a prompt; it returns `version`, `path`, `logged_in` and `auth`, and fails when Claude Code reports that nobody is signed in.

## Check allowlist

The evaluate step runs only the `{cmd, args}` pairs listed in `plan_meta.checks`, with no shell. `cmd` must be one of `test`, `[`, `true`, `false`, `cat`, `grep`, `egrep`, `fgrep`, `ls`, `wc`, `head`, `tail`, `diff`, `cmp`, `stat`, `sha256sum`, `md5sum`, `file`, with no path separator (`verify.rs`).

## Conventions

- A field that is not in the code is not written. A new field needs a new schema number.
- The backend never deletes a task record.
- The app never writes `state` (except done by hand), `step`, `answers`, `assertions`, `verdict`, `plan_meta`, `cost_usd`, `links`, `## Log` or `approval.digest`. It may write `approval.status`, `by`, `at` and `note`.

## Fixtures

`fixtures/` holds twelve complete trees: 01-reminder, 02-open-question, 03-choice-question, 04-awaiting-approval, 05-running, 06-waiting-mid-run, 07-succeeded, 08-failed, 09-redelegated-pair, 10-registry, 11-paused and 12-suspended. Their records are `schema: 4`, which tests the upgrade path to 7.
