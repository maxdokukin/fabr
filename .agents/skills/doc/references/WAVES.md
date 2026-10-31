# Waves

Four waves, poured in order. Wave 1 is the foundation. The rule that protects it: a later wave fills slots that wave 1 left and does not change a file format. A format change needs the owner and a new schema number. Schema 4 shows the rule bends, always by owner decision (see `DECISIONS.md`).

Where a feature goes, the first yes wins.

- It changes a format or a state: wave 1, or the owner.
- One task cannot run end to end without it: wave 1.
- It makes a second run better than the first: wave 2.
- It lets the agent act with less supervision: wave 3.
- It adds a channel or a connector: wave 4.

## Wave 1: foundation

Goal: one task goes from reminder to succeeded through Telegram, with a grant, a fill and an audit entry, and a restart mid-run resumes.

| Item | Status |
|---|---|
| Settings, secrets, setup wizard with a Claude Code check | built |
| Task record: six states, flags, atomic writes with flock, validation, schema 7 with session ids | built |
| GUI: tree, delegate, answers, approval, agents sheet, sessions and transcripts | built |
| Orchestrator with resume, one running task per node, registry and heartbeat | built |
| Planner, executor and validator as Claude Code sessions | built |
| Notifier: events log and Telegram notices | built |
| Context gatherer: folder chain, rules, knowledge, learned answers | built |
| Telegram bot, transport and buttons | built, tested against a mock only |
| Task index | planned; the app polls a stamp |
| Knowledge files with editors in node settings | built |
| Data bank, fill at the sink, audit log, payment refusal | planned |
| Learner (capture only) | built |

Exit test on a fresh machine: finish setup, hand-write a reminder and delegate it. Answer one batched question on Telegram, then approve plan, assertions and grants in one tap. The executor fills address fields on a web form and every assertion passes. The notice arrives on Telegram and in the app. A kill during execute resumes, and the audit log has one line per filled field.

## Wave 2: tell it once

Built on 2026-10-31: the once, always or no prompt, always-rules applied silently and listed in the approval, and the app managing learned records. Similar finished tasks in the folder chain are matched by title words and shown to the planner. Not built: grant scope allow-for-this-project, grant revocation, full-text search over the bodies of past records and per task-type checklists.

A decision made in one task is offered back in the next, with the three answers once, always, or no. Learned records are read by the gatherer and shown to the definer as priors with an applies-when condition. Always-rules apply silently and are listed in the approval message. The app manages learned records. Grant scope allow-for-this-project, grant revocation, full-text search over past records and per task-type checklists come here. Exit test: the same kind of task three times, asked, then always, then assumed and listed.

## Wave 3: trust and control

Built on 2026-10-31: a failed evaluation re-plans once, cost caps per run and per task, auto mode limited to local file rules, promotion of an always-rule to global after two projects, and the staleness review of always-rules. Procedure proposals are built: the executor session proposes after a pass and the user accepts in node settings. Not built: checkpoints inside a run, several agents per folder and long-form review.

Procedure proposals are staged after a passed evaluation and accepted in the app, never auto-accepted. A learned rule is promoted to global after confirmation in two projects. Always-rules get a staleness review. An irreversible step waits at a checkpoint even in auto mode. A failed evaluation re-plans once. Cost caps, several agents per folder and long-form review in the app belong here. Exit test: a run proposes a procedure, the user accepts it, the next run uses it, and an irreversible task waits at a checkpoint.

## Wave 4: reach

Wave 4 adds speech and photo input, and personal context connectors (email, calendar, messages) as read-only tools behind a grant that states the filter. It adds virtual cards per task, so the payment refusal can lift for pre-approved amounts, and screenshot redaction. Several machines may share a global layer. Embeddings come only if full-text search stops being enough. Exit test: a voice note becomes a reminder, a task reads granted emails and nothing else, and a purchase completes without the card number in any transcript.
