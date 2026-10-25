# Demo

Run on 2026-10-25 from `unified-integration`. The agent was `cargo run -p fabr -- agent --once` with `FABR_HOME` pointed at a temporary home, so the user's real default folder was not driven. Secrets stayed in the existing secrets file through a symlink outside the repo. No secret values were printed.

## Task A

Record `2026-10-25-internship-budget`. Claude defined and planned it. Approval named `propose_memory`, `confirm_memory`, and `recall_preferences`. Execution wrote the local preference file and the log contains:

`recall_preferences source=local count=1 ... text=Stay under $100 for internship supplies`

The task reached `succeeded`.

## Task B

Record `2026-10-25-desk-setup`, in the same folder, after Task A. The plan bound one `bright_data_search` call to the public query `basic desk setup for students desk chair lamp monitor price`, plus local list, read, recall, and `write_file` of `desk.md`. The task stayed at `approve` until that plan was approved with an `external_action` grant. The log then shows the search `succeeded` with `hits=8`, then `write_file desk.md`, then `verdict pass`.

`desk.md` repeats the recalled budget and says nothing was purchased. It also says the eight hits did not include prices, and that the dollar figures in the file are estimates.

## Separate checks

- Nessie: `live_nessie_task_is_labeled_sandbox` passed. The log says `nessie sandbox` and `accounts=0`. It does not read the internship preference file.
- Scope: `out_of_root_driven` passed. A symlink whose target is outside the search root is not driven, and the in-root record still is. `scope_containment_prop` passed.

## Cartesia

`live_cartesia_task_writes_after_approval` wrote `speech.wav` after approval. This desktop pass did not record from the microphone. The record sheet has Record and Pick audio, which save `capture.wav` in the node, and Play speech, which reads `speech.wav` only after the task log contains `cartesia_speak` and `succeeded`.

## App build

`npm run tauri build` compiled `target/release/fabr-desktop` and started `Fabr.app`. The `.dmg` step failed because Finder timed out while attaching the disk image. The app binary is the launchable build.
