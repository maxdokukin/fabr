# Fabr

Fabr turns the reminders next to your files into tasks that Claude Code plans, runs and checks in that folder. It is for people who already use Claude Code and organise their work in folders.

## Problem

Your reminders live in one app, and your Claude Code sessions live in a terminal history. To hand a reminder to Claude Code, you open a terminal in the folder and retype the context. A week later you cannot tell which session did which task, or whether anyone checked the result.

## Solution

Fabr keeps every task as a Markdown file inside the folder it belongs to, and shows those folders as a tree. Press Delegate and fabr starts Claude Code in that folder three times. A planner session writes a plan with inputs, outputs and steps, and you approve it. An executor session carries it out. A validator session checks the result against the assertions you approved. Three ideas hold it together.

1. The file tree is the spine. Tasks, rules and knowledge are keyed by folder path, and a folder inherits from the folders above it.
2. Every task records its Claude Code session ids, so you can resume the planner, executor or validator in a terminal at any time.
3. You keep the wheel. The executor may edit only inside the task's folder with the tools you approved, and anything else comes back to you as a question.

## See it

After a run, the task file holds the plan, the verdict and the three sessions:

```
sessions:
  plan: 0c15bc0c-4af9-4ba7-b36c-2a466a6ca85f
  execute: 087a2a84-8881-4c81-82f5-0ff7d74a11a9
  evaluate: 1ea0d2e5-52be-4d83-90d1-7daff22ce8f1
## Log
- 06:52:26Z execute: started
- 06:52:31Z execute: Read meeting-2026-10-29.md
- 06:52:34Z execute: Write action-items.md
- 06:52:45Z evaluate: verdict pass
```

## Why not Claude Code alone

| | Claude Code in a terminal | Fabr |
|---|---|---|
| Where the task lives | the chat history | a Markdown file in your folder |
| What gets approved | each tool call, one prompt at a time | one plan, its tools and its checks |
| Who checks the result | you | a separate validator session, then you |

## Install

Install Claude Code from https://claude.com/claude-code and sign in once by running `claude`. Then download the macOS build from https://github.com/maxdokukin/fabr/releases, open the `.dmg` and drag Fabr to Applications. The build is unsigned, so clear the quarantine flag once:

```sh
xattr -d com.apple.quarantine /Applications/Fabr.app
```

Open Fabr. Setup asks for a default folder and checks that Claude Code is installed and signed in.

## Usage

Add a reminder: type a title on a folder in the tree, press Enter and fill the sheet.

Delegate it: press Generate with AI. Answer the planner's question if it asks one, then read the plan and approve it. The node's stage bar follows the task to the verdict.

Open a session: in the task sheet, press Open in terminal next to the planner, executor or validator. Press Transcript to read the session in the app instead. Node settings list every Claude Code session in that folder, plus its rules and knowledge.

Work from a terminal: `fabr status` lists every task and what it waits for, and `fabr approve <task>` approves a plan.

## Status

Planning, approval, execution and validation run through Claude Code today, with spend caps and a time limit. An answer you mark Always applies to later tasks in that folder, and a failed check gets one revised plan. Telegram questions, approvals and notices are built and tested against a mock only. The personal data bank and form filling are designed and not built.

---

## Development

```sh
cargo test --workspace
cd app && npm install && npx vitest run
cargo run -p fabr -- serve        # web UI on loopback
cargo run -p fabr -- agent        # the runner, without the desktop app
cd app && npm run tauri build     # macOS bundles
```

## Layout

`crates/fabr-core` holds the tree, the task record format, setup, Telegram and the ssh backend. `crates/fabr-agent` is the pipeline runner and the Claude Code engine, see [crates/fabr-agent/README.md](crates/fabr-agent/README.md). `crates/fabr` is the command line and the loopback server. `app` is the Svelte and Tauri desktop app. `fixtures` holds one record per state, see [fixtures/README.md](fixtures/README.md). `sample-fabr-space` is the demo dataset, see [sample-fabr-space/README.md](sample-fabr-space/README.md).

## License

GPL-3.0-only, see [LICENSE.txt](LICENSE.txt).
