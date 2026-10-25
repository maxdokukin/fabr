# Sponsor tracks

Eligibility is unverified for every row. A passing adapter test is not a judging result. Credential field names are the canonical names; aliases in parentheses are accepted from the secrets file.

| Track | What the user gets | What the code does | Source | Test | Credential | Evidence | Eligibility |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Anthropic | Claude plans and runs each step | Existing model client. No second planner. | `crates/fabr-agent/src/model.rs` | Task A and Task B on 2026-10-25 | `anthropic_api_key` | Both demo tasks reached `succeeded` | unverified |
| Microsoft | none | No metrics client | none | none | none | not integrated | unverified |
| Moss | Optional index lookup | `moss_list` and `moss_retrieve` after approval. `moss_upload` does not run. | `crates/fabr-agent/src/connectors/moss.rs` | `cargo test -p fabr-agent --test sponsor_live -- --ignored` | `moss_project_id` (`MOSS_PROJECT_ID`), `moss_project_key` (`MOSS_PROJECT_KEY`) | `indexes=3`, `docs=75` on `agent-memory-context` | unverified |
| Fo | none until a public read exists | Discovery only. Allowlist is empty. | `crates/fabr-agent/src/connectors/fo.rs` | same sponsor_live command | `fo_api_key` (`fo_api_ke`) | `tools=5`, `allowlist=0` | unverified |
| Cartesia | Speech in the task folder | `cartesia_speak` writes `speech.wav`. `cartesia_transcribe` reads a node path. The sheet can save `capture.wav` and play `speech.wav` after success. | `crates/fabr-agent/src/connectors/cartesia.rs`, `app/src/lib/fabric/SpeechCapture.svelte` | sponsor_live Cartesia tests | `cartesia_api_key` | `voices=1049`, `bytes=22050`, `chars=5` | unverified |
| Apify | One Actor call | `apify_lookup` runs the named Actor after approval | `crates/fabr-agent/src/connectors/apify.rs` | `live_apify_task_runs_after_approval` | `apify_api_token` (`apify_api_key`) | `apify~hello-world items=1` | unverified |
| Bright Data | Public search and page extract | `bright_data_search` and `bright_data_extract` after a bound approval | `crates/fabr-agent/src/connectors/brightdata.rs` | `cargo test -p fabr-agent --test bright_data -- --ignored` | `brightdata_api_token` | docs test passed; Task B `hits=8` | unverified |
| Visa | none | `visa_checkout` is not executable. No cert paths. | `crates/fabr-agent/src/tools_contract.rs` | none | `visa_cert_path`, `visa_key_path` | not integrated | unverified |
| Capital One Nessie | Labeled sandbox account read | `nessie_sandbox` after a `purchase_finance` grant | `crates/fabr-agent/src/connectors/nessie.rs` | `live_nessie_task_is_labeled_sandbox` | `nessie_api_key` (`nessi_api_key`) | `accounts=0` | unverified |
| Figma | none | No client | none | none | none | not integrated | unverified |
| Atlassian Forge | none | No Forge app | none | none | none | not integrated | unverified |

Commands above were run on this branch. Live tests are `#[ignore]` and were passed `--ignored`.
