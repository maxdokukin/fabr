# Fabr

Fabr turns the todos next to your files into tasks that an agent defines, plans, runs and checks. It is for people who already organise their work in folders.

## Problem

Your todos live in one app and the files they are about live in another. Handing a task to an AI agent means retyping the context, because the agent cannot see your folder. Once it runs you cannot tell which step it is on, and the only way to stop it is to kill it.

## Solution

Fabr keeps every task as a Markdown file inside the folder it belongs to, and shows those folders as a tree. You type a title and press Enter to add a todo. Press Delegate and a fixed pipeline takes it from a one-line reminder to a checked result. It gathers the folder's context, defines the task and asks you what is missing. Then it plans, waits for your approval, executes, and evaluates the result against the assertions you approved. Three ideas hold it together.

1. The file tree is the spine. Tasks, rules and knowledge are keyed by folder path, and a folder inherits from the folders above it.
2. The outer loop is code and the model works inside one step at a time, so every gate can be tested and replayed.
3. You keep the wheel. You can pause, suspend, complete or cancel a task at any time, and edit any section of it until execution starts.

## See it

```sh
fabr add ~/notes "write the release notes" --due 2026-11-01
head -8 ~/notes/.fabric-space/tasks/2026-10-25-write-the-release-notes.md
```

```
---
schema: 6
id: 2026-10-25-write-the-release-notes
source: null
title: write the release notes
due: 2026-11-01
pri: 0
state: reminder
```

## Why not a todo app or a coding agent

| | A todo app | A coding agent in a terminal | Fabr |
|---|---|---|---|
| Where the task lives | its own database | the chat scroll | a Markdown file in your folder |
| Who can read it later | you | nobody | you, the agent, and `git diff` |
| Stopping a run | not applicable | Ctrl-C | pause, suspend, complete or cancel in one click |
| What the agent may touch | nothing | anything on the machine | the task's folder, with 4 tools |

## Install

Download the macOS build from https://github.com/maxdokukin/fabr/releases, open the `.dmg` and drag Fabr to Applications. The build is unsigned, so clear the quarantine flag once:

```sh
xattr -d com.apple.quarantine /Applications/Fabr.app
```

Open Fabr. A 6-screen setup asks for a default folder, an Anthropic API key and a model. The agent runs in the background while the app is open.

## Usage

Add a todo in the folder it belongs to: type a title on the node, press Enter and fill the sheet. Save writes a reminder.

Delegate it: press Generate with AI in the sheet. The node shows a stage bar as the agent moves through the 9 steps, and the Agents badge in the toolbar lists every running agent.

Follow folders on another machine: add an ssh source in Settings. You open, run and edit the tasks on that machine from your Mac over the ssh connection you already have.

## Status

The pipeline runs end to end today. Define, plan and approval work, with an auto mode that skips the tap. The tool loop has 4 tools that stay inside the task's folder: read any file, list, write Markdown, make a folder. The evaluator is a model with a fresh context that judges each assertion from the evidence. Pause, suspend, complete, cancel, redo and per-section edits work, and ssh sources are read and written.

## Sponsors

Sponsor tools are optional. They appear when `~/.fabric-space/secrets.toml` parses and contains that provider's key. An approval with an empty `bound_actions` list still runs the local read, list, Markdown write, and make-directory tools. It does not authorize a sponsor tool. A remote call needs its own grant. The record sheet shows each bound action's tool, destination, effect, and grant. A log line that says `UNKNOWN` is a failure waiting for reconciliation.

What ran on `unified-integration` on 2026-10-25:

- Task A confirmed `Stay under $100 for internship supplies` into the local preference file.
- Task B recalled that line, searched once with Bright Data after approval (`hits=8`), and wrote `desk.md`. The prices in that file are estimates. The search did not return prices. Nothing was purchased.
- Bright Data's docs test wrote `research.md` with `source: bright_data` and `https://docs.rs/ureq` only after approval.
- Moss listed 3 indexes and retrieved 75 documents from `agent-memory-context`. `moss_upload` does not run.
- Cartesia wrote `speech.wav` (`bytes=22050`) and transcribed it (`chars=5`). The record sheet can record or pick `capture.wav` and plays `speech.wav` only after an approved speak run succeeds.
- Apify ran `apify~hello-world` and returned one dataset item.
- Nessie read sandbox accounts (`accounts=0`), labeled sandbox.

Fo discovered five private account tools and called none. Visa, Figma, Microsoft, and Atlassian Forge are not integrated. Anthropic is the model client. Track eligibility is unverified. Details: [docs/SPONSOR_TRACKS.md](docs/SPONSOR_TRACKS.md), [docs/DEMO.md](docs/DEMO.md), [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md), [docs/TEST_RESULTS.md](docs/TEST_RESULTS.md), [docs/sponsor-evidence.md](docs/sponsor-evidence.md).

Next, execution moves to an isolated sandbox that runs a coding harness with a fixed tool set. Learning from answers, Telegram and the personal data bank are designed and not built.

---

## Development

```sh
cargo test --workspace
cd app && npm install && npm test
cargo run -p fabr -- serve        # web UI on loopback
cd app && npm run tauri build     # macOS bundles
```

## Layout

`crates/fabr-core` holds the tree, the format of a task record and the ssh backend. `crates/fabr-agent` is the pipeline runner. `crates/fabr` is the command line and the loopback server. `app` is the Svelte and Tauri desktop app. `fixtures` holds one record per state, see [fixtures/README.md](fixtures/README.md). `sample-fabr-space` is the demo dataset, see [sample-fabr-space/README.md](sample-fabr-space/README.md).

## License

MIT, see [LICENSE.txt](LICENSE.txt).
