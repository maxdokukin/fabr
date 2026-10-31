# Architecture

Fabric takes a line dumped into a folder and turns it into a task. Claude Code, installed on the machine, does the thinking: one session plans the task, a second runs the plan, and a third checks the result, and the user approves in between. Fabric is the aggregator around those sessions. It keeps each task, its plan and its session ids in the folder the task belongs to, so reminders and Claude Code sessions live in one place. This file is the big picture: the layers, the status legend, the 26 blocks with what is built today, and what the app looks like.

## Status legend

| Status | Meaning |
|---|---|
| built | in code today |
| stub | the slot, name and step exist; the body is a placeholder |
| planned | designed here, no code |
| parallel | owned by the input project that runs alongside; only a file contract is shared |
| named | drawn on the map so nobody fills the hole by accident; never scheduled |

## Layers

| Layer | Holds |
|---|---|
| Channels | how the user touches it |
| Core | the record store, the sequencer, the registry, the notifier |
| Agents | Claude Code sessions with one job each: planner, executor, validator |
| Knowledge | files the agents read, merged by precedence: nearer folder wins, project over global |
| Private data and tools | where values and side effects live |

Everything above the knowledge layer reads and writes the task record in the folder. The context gatherer is not a model: it walks the folder chain and merges knowledge. The executor is the only agent with side effects, and it runs with edits allowed inside the task's folder only. The data bank hands values to fill, never to an agent, and every fill lands in the audit log. The orchestrator is the only sequencer and the only writer of `state`, except the done-by-hand exception in `CONTRACT.md`.

## The 26 blocks

| Layer | Block | Status | Where it lives, or what is missing |
|---|---|---|---|
| Channels | First-run setup | built | `fabr-core/src/setup.rs`, `app/src/lib/setup/`; folder, Claude Code (version and sign-in), optional Telegram |
| Channels | Input capture | parallel | the app has an add-task line and sheet; speech and photo are wave 4 |
| Channels | Local GUI | built | `app/`, a Svelte 5 and Tauri 2 desktop app |
| Channels | Telegram bot | built | `fabr-core/src/telegram.rs`, driven from `runner/mod.rs`; untested against the real Bot API |
| Core | Task record store | built | `fabr-core/src/taskrec.rs`, per-folder flock, schema 7 |
| Core | Task index | planned | no sqlite; the app polls a stamp over the tree |
| Core | Orchestrator | built | `fabr-agent/src/lifecycle.rs`, hosted in the desktop app |
| Core | Agent registry | built | `fabr-agent/src/board.rs`, heartbeat in `runner/` |
| Core | Notifier | built | `events.log` plus Telegram notices |
| Agents | Organizer | parallel | not in this repo |
| Agents | Context gatherer | built | `gather_bundle` in `lifecycle.rs`: folder chain, rules, knowledge, learned answers, file listing |
| Agents | Definer | built | planner session, `claude.rs` `define` |
| Agents | Planner | built | the same session resumed, `claude.rs` `plan`; Inputs, Outputs, Steps |
| Agents | Executor | built | executor session, `claude.rs` `execute`, acceptEdits in the task folder |
| Agents | Evaluator | built | allowlisted checks, then a read-only validator session |
| Agents | Learner | built | `fabr-core/src/knowledge.rs` `capture`: answers to `learned.yaml`; no classification into preference, data or procedure |
| Knowledge | Machine settings | built | `settings.toml`, `secrets.toml` (bot token only) |
| Knowledge | Global rules and preferences | built | `.fabric-space/.agents/rules.md` and `knowledge.md` at the root, edited in node settings |
| Knowledge | Project knowledge and preferences | built | the same files in any folder; nearer folder listed first |
| Knowledge | Learned preferences | built | the planner cites a learned record; define asks once, always or no; always-rules apply silently and show under Assumed |
| Knowledge | Past executions | planned | wave 2 search over finished records |
| Private | Personal data bank | planned | no vault, policy or crypto |
| Private | Fill at the sink | planned | Claude Code has WebFetch; no form filling with granted values |
| Private | Audit log | planned | comes with fill |
| Private | Tool set | built | Claude Code's own tools, limited per task by `plan_meta.tools` |
| Private | Personal context connectors | named | never built in wave 1 |

## The app

- The tree is the only main view. A folder node shows task counts and a waiting or running mark, and clicking into it shows its tasks, panels and logs. There is no Board or Tabs view.
- A node card has a gear that opens node settings. The side panel holds the same settings body.
- Tasks are created from the node panel: an "Add task" line opens a sheet with title, due, priority, context, spec and success. A reminder is also what the UI labels a todo.
- A stage bar for each node and each task, and an Agents sheet opened from the toolbar badge, show progress. Both are fed by the record's `step` and the registry entry.
- Remote ssh nodes list their tasks read-only through the snapshot.
- The app hosts the agent runner in-process. It starts when setup has the `folder` and `claude` steps and stops on exit, which also kills any running Claude Code process. `fabr agent` runs the same runner as a debug entry point.
- A task sheet lists its planner, executor and validator sessions with Open in terminal (`claude --resume <id>` in the task's folder), Copy command and Transcript.
- The transcript sheet has a read-only chat box that resumes the session headlessly, so the user can discuss a plan with its own planner and then press Redo plan.
- The CLI covers the same loop for terminal users: `fabr status`, `show`, `delegate`, `answer`, `approve`, `refuse`, `cancel`, `pause` and `resume` write through the GUI's write path with `by: cli`, and `fabr ask <task> --role plan|execute|evaluate` puts a read-only question to one of its sessions.
- Node settings list every Claude Code session that ran in that folder, read from `~/.claude/projects/`, with a New session here button, and hold the folder's rules, knowledge and learned answers.
- The frontend parses records in one place, `app/src/lib/fabric/parse.ts`, and tests against `fixtures/`.
