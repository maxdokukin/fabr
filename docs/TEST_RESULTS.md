# Test results

Branch `unified-integration`, 2026-10-25. Commands were run in `/Users/bradleyharaguchi/fabr-unified`.

| Command | Result |
| --- | --- |
| `cargo test --workspace --offline` | passed, including ignored tests left ignored |
| `cargo fmt --all --check` | passed again after the planner learned to keep `bound_actions` |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | passed again after that change |
| `cd app && npm test` | 508 passed |
| `cargo test -p fabr-agent --test out_of_root_driven --test scope_containment_prop --test tool_contract --offline` | 1 + 1 + 6 passed. `legacy_approval_does_not_run_a_sponsor_action` is in `tool_contract` |
| `cargo test -p fabr-agent --test sponsor_live -- --ignored --test-threads=1` | 9 passed |
| `cargo test -p fabr-agent --test bright_data -- --ignored` | `live_public_docs_and_issues` passed |
| `npm run tauri build` | `target/release/fabr-desktop` built. The `.dmg` bundle failed: Finder AppleEvent timed out |
| `FABR_HOME=/tmp/fabr-demo-home ./target/release/fabr-desktop` | process started and the agent logged `default_folder=/tmp/fabr-unified-demo`. Screen Recording was off, so the window was not captured |

Sponsor status lines from the ignored run:

- `moss: ok moss_list indexes=3 names=agent-memory-context,languages,support-docs`
- `moss-retrieve: ok moss_retrieve index=agent-memory-context docs=75 source=moss`
- `fo: ok tools=5 names=ask_fo,get_fo_conversation,get_fo_reply,list_fo_conversations,start_fo_call allowlist=0`
- `cartesia-voices: ok cartesia voices=1049 first_voice_present=true`
- `cartesia-tts: ok cartesia_speak bytes=22050 path=speech.wav`
- `cartesia-stt: ok cartesia_transcribe chars=5`
- `apify: ok apify_lookup actor=apify~hello-world items=1`
- `nessie: ok nessie sandbox accounts=0`
- Bright Data: `live_public_docs_and_issues` passed in 14.95s

The lifecycle tests in that file also passed: Moss, Cartesia, Apify, and Nessie each ran only after approval.

## Blockers

- Fo's five tools are private account actions, so `fo_action` is not callable.
- Visa has no cert paths and no sandbox call.
- Figma, Atlassian Forge, and a Microsoft metrics client are not in the tree.
- `moss_upload` is not executable.
- Apify evidence is the hello-world Actor, one item, not a product comparison.
- Bright Data Task B returned 8 hits and no prices. `desk.md` says its prices are estimates.
- The microphone was not used in the desktop demo. Capture and playback are in the record sheet.
- Public Benefit community, hackathon-history, and every sponsor judging requirement are unverified.
- `npm run tauri build` did not produce a `.dmg` because Finder timed out.
- Nothing was pushed or tagged.
