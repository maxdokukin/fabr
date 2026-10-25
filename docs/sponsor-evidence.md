# Sponsor evidence

Re-checked on 2026-10-25 after `~/.fabric-space/secrets.toml` parsed again. `apify_api_key` is an alias of `apify_api_token`. Ignored live tests in `tests/sponsor_live.rs` and `tests/bright_data.rs` passed. Nothing was published, tagged, or pushed. Secret values are not in this file. The branch was `sponsor-integration` at `63ec80d` before these notes.

Labels:

- **Adapter working** — a standalone authenticated call succeeded from this worktree.
- **FABR integrated** — the tool ran through plan, approval, and execution on a task record.
- **Track qualified** — judging requirements for that sponsor are met. This port has no judging evidence, so every row stays unverified.

## Matrix

| Sponsor | Adapter | FABR lifecycle | Track | Blocker |
| --- | --- | --- | --- | --- |
| Anthropic | not re-probed here | already the model client on v0.7.0 | unverified | Public Benefit community and measurable impact were not checked. |
| Microsoft | no | no | unverified | No Microsoft client was added. |
| Moss | yes | yes, for list | unverified | `listIndexes` returned 3 indexes. `getDocs` on `agent-memory-context` returned 75 documents and did not write `learned-preferences.json`. `moss_list` waited for approval and an `external_action` grant, then ran. `moss_upload` is not executable. Local learned preferences stay authoritative. Upload still needs a separate `external_upload` grant and a real upload call. |
| Fo | yes, discovery only | no | unverified | `https://mcp.api.wajo.ai/fo` accepted the bearer token. `tools/list` returned `ask_fo`, `get_fo_conversation`, `get_fo_reply`, `list_fo_conversations`, `start_fo_call`. None is a narrow public read, so the allowlist stays empty and no tool was called. `ask_fo` sends a request that Fo acts on from the user's account. `get_fo_reply` polls that in-progress action. `list_fo_conversations` and `get_fo_conversation` read private conversations. `start_fo_call` starts a live voice session. `fo_action` stays not executable. |
| Cartesia | yes | yes, for a bound file write | unverified | Voices, TTS (`sonic-2`), and batch STT (`ink-whisper`) succeeded. A task approved `cartesia_speak` and the backend wrote `speech.wav` inside the node. The record sheet can save `capture.wav` and plays `speech.wav` only after that approved run succeeds. |
| Apify | yes | yes | unverified | `apify~hello-world` returned `items=1`. `live_apify_task_runs_after_approval` ran `apify_lookup` only after approval and an `external_action` grant. This is the hello-world Actor, not a shopping comparison. |
| Bright Data | yes | yes | unverified | `live_public_docs_and_issues` passed again. The task stayed pending until approval, then `research.md` contained `source: bright_data` and `https://docs.rs/ureq`. The token was not in the note or the log. That is a public docs page, not a priced product comparison. |
| Visa | no | no | unverified | Re-checked through the loader and the environment: no `visa_cert_path` or `visa_key_path`. No sandbox call was made. |
| Nessie | yes | yes | unverified | `GET /accounts` returned a labeled sandbox result (`accounts=0`) and did not touch internship memory. The task path required `purchase_finance` and a bound call. |
| Figma | no | no | unverified | No client and no slides. |

## Re-check

Ignored tests passed with these status lines and no secret values:

- Moss: `indexes=3` named `agent-memory-context`, `languages`, `support-docs`. Retrieve returned `docs=75` and `source=moss`. `live_moss_task_lists_after_approval` passed.
- Fo: `tools=5` (`ask_fo`, `get_fo_conversation`, `get_fo_reply`, `list_fo_conversations`, `start_fo_call`), `allowlist=0`.
- Cartesia: `voices=1049`, TTS `bytes=22050` at `speech.wav`, STT `chars=5`. `live_cartesia_task_writes_after_approval` passed. An earlier run on `sponsor-integration` reported `bytes=23822`.
- Nessie: `accounts=0`. `live_nessie_task_is_labeled_sandbox` passed.
- Apify: `actor=apify~hello-world items=1`. `live_apify_task_runs_after_approval` passed.
- Bright Data: `live_public_docs_and_issues` passed. See `docs/brightdata-live.md`.

The same ignored tests passed again on `unified-integration` on 2026-10-25. A separate demo, with `FABR_HOME` pointed at a temporary home, confirmed the internship preference and then wrote `desk.md` after one approved Bright Data search (`hits=8`). The prices in `desk.md` are labeled estimates because the search hits did not include prices. Out-of-root records stayed undriven in `out_of_root_driven`. Nessie stayed a labeled sandbox (`accounts=0`).

## Rebase

`origin/main` moved from `bdfb585` to `460d4d6`. That commit only adds the root README. `git rebase origin/main` finished with no conflict files. `cargo fmt --all --check`, `cargo clippy -p fabr-agent --locked --all-targets -- -D warnings`, and `cargo test -p fabr-agent --locked` passed. The lockfile was not regenerated.

## Approval and UNKNOWN

`bound_actions` is on schema 6 `PlanMeta` with `#[serde(default)]`. The approval digest includes them only when the list is non-empty, so an approval with no bound actions keeps its digest. That approval still authorizes the legacy local tools and does not authorize a sponsor tool. A sponsor call must match tool, canonical params, destination, effect, and grant. A different URL, recipient, or body does not match.

Timeout and `unavailable` are `UNKNOWN`: no replay, no pass verdict, and no later tool in that batch. Web pages and MCP tool text are stored as data. They do not change grants, approvals, or the tool list.

Sponsor file writes go through a stricter path check than a plain read. A symlink, an absolute path, or a missing parent is not written. Existing demo scope checks are unchanged.

## Left untouched

`bradley` stayed at `14655bd`. `fabr-main` stayed detached at `8a5330b`. The stash `cursor-wip-and-sample-drift-pre-rebase-20261025-045134` was not popped or dropped. `runner/mod.rs`, `App.svelte`, `tasks.svelte.ts`, and `view.ts` were not edited. The GUI serializer still omits `bound_actions`; see `docs/sponsor-frontend.md`.
