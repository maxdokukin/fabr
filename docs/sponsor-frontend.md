# Sponsor frontend wiring

The backend on `sponsor-integration` is the seam. The desktop UI does not show sponsor approval or Cartesia capture yet.

## Backend

`Ctl.tools` is a `WorkspaceTools` value. It implements `ToolSet`: demo tools (`read_file`, `list_dir`, `write_file`, `make_dir`) plus sponsor tools whose credentials are present. Disabled tools are absent from `specs()`, so the execute request cannot call them. The plan prompt still lists executable tools as enabled or disabled, with input schemas and no secret values or secret paths.

`PlanMeta.bound_actions` (schema 6, default empty) is the approval binding: tool, canonical params, destination, effect, and grant class. The digest includes that list only when it is non-empty. Execute checks the binding and the grant, then calls `ToolSet::call`. Remote reads are egress. A local read grant does not authorize Bright Data, Moss, Fo, Apify, or Cartesia. `UNKNOWN` external results fail the verdict and are not replayed.

## Frontend, in this order

1. Parse `bound_actions` on `PlanMeta` in `app/src/lib/fabric/types.ts`. Today that type has reversibility, tools, requires, assumptions, and checks only.
2. Round-trip the list in `app/src/lib/fabric/serialize.ts`. The `plan_meta` writer currently emits reversibility, tools, requires, assumptions, and checks. A GUI save of a sponsor task drops `bound_actions`. That is a behavior bug once the UI writes records; it is not fixed in this port.
3. On the approve step, show each bound action's tool, destination, and effect, and the grant field the call needs (`external_action`, `external_upload`, or `purchase_finance`). Approving still sets the existing approval status. Do not put secret values on screen.
4. On the result, show an external `UNKNOWN` line from the task log as failed and waiting for reconciliation. Do not route that through the notice path, and do not change scope visibility.
5. Cartesia capture belongs in the app, not in a headless button. Record or pick an audio file inside the node, create a task whose bound action is `cartesia_transcribe` for that path, and play `cartesia_speak` output only after approval. The backend can already write `speech.wav` inside the node. That file write is not the voice demo.

`App.svelte`, `tasks.svelte.ts`, and `view.ts` were left unchanged so notice routing and scope visibility stay as they are on v0.7.0.
