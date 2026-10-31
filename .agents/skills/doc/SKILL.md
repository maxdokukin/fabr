---
name: doc
description: >
  Vision, design philosophy and format contract of fabric, the folder-based system that turns a
  dumped reminder into a task that local Claude Code sessions plan, run and check. Holds the
  owner's founding input, the twelve design principles, the decisions log, the architecture with a
  built or planned mark for each of the 26 blocks, the pipeline steps, their gates and the claude
  flags per role, the file contract (task record schema 7 with session ids, states, requests,
  registry, events, settings, knowledge files, check allowlist), the Telegram wording, the four
  feature waves, and the Claude Code CLI gotchas and test commands. Read it for anything about architecture, the task
  lifecycle, grants, fill at the sink, learned preferences, waves, or why a thing is the way it
  is. The code is the source of truth: these files give intent and contracts, and may lag it.
---

# doc

This is the index of the project's agent knowledge. Open only the file whose row matches the task. Each row names one file under `references/`, and every file there has a row. Where a file and the code disagree, the code wins; fix the file.

| File | Read it when |
|---|---|
| `references/USER_INPUT.md` | you need the owner's own words on what fabric is and must do, from the founding conversation of 2026-10-24. Frozen: never edit |
| `references/PHILOSOPHY.md` | shaping anything new, so it fits the twelve principles |
| `references/DECISIONS.md` | changing anything a past decision covers. Append-only: a new dated line supersedes an old one by naming it |
| `references/ARCHITECTURE.md` | you need the big picture: the layers, the status legend, the 26 blocks with what is built, and what the app shows |
| `references/PIPELINE.md` | touching what happens after delegate: the eight steps, their gates, the rules the code enforces, restart and resume |
| `references/CONTRACT.md` | anything that crosses between the backend, the core and the app: paths, record fields and writers, states, requests, questions, approval, registry, events, settings, setup checks, fixtures |
| `references/TELEGRAM.md` | writing or changing what the Telegram bot says or how it answers |
| `references/CLAUDE_CODE.md` | changing how fabr runs `claude`, reading its session files, or testing against a fake or the real CLI |
| `references/WAVES.md` | deciding whether a feature belongs now or later, and what wave 1 has and has not built |
