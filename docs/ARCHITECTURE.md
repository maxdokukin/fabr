# Architecture

Fabr keeps one task record per task, as Markdown under `<folder>/.fabric-space/tasks/`. That file is the shared state. The outer loop is gather, define, plan, approve, execute, evaluate, learn, notify. Claude is the planner and the executor inside a step. Sponsor adapters do not write the approval or the task state.

## Tools

`crates/fabr-agent/src/toolset/` is the v0.7.0 tool seam. The demo tools are read, list, Markdown write, and make a directory. They resolve paths inside the task's folder. `WorkspaceTools` in `sponsor_tools.rs` adds sponsor tools whose credentials are present and leaves the others out of the executable list.

A registered tool is not authorized. `tools_contract.rs` requires the tool name on the approved plan. Sponsor tools also need a bound action: tool, canonical params, destination, effect, and grant class. Local read does not authorize a remote call. `moss_upload`, `fo_action`, and `visa_checkout` stay non-executable.

An empty `bound_actions` list is omitted from the approval digest, so older approvals keep their hash. Those approvals still cannot run a sponsor tool. A timeout or a missing external result is `UNKNOWN`. It is not replayed, it fails evaluation, and it does not count as success.

## Scope and notices

`default_folder` is a search root. `Runner::records` keeps a record whose canonical path is inside that root and drops one that resolves outside it. `start` warns once per dropped record. A path that cannot be canonicalized stays in the list so a transient stat failure does not hide a real record. The same `bad_records` set is used for that warning and for a record that fails to parse.

The desktop resolves a notice through `findByTaskPath`, including a redelegated successor, and opens the record even when its folder is not a tree node. An unreadable file gets a different message from a missing file.

## Memory

`learned-preferences.json` in the fabric space is the preference store. `propose_memory` does not become recallable until `confirm_memory`. Moss list and retrieve are a separate index. They do not replace that file.

## Desktop

The Tauri app starts the same runner. The record sheet round-trips `bound_actions` in the Markdown serializer and shows them on the approval panel. Speech capture writes only `capture.wav` in the node. Playback reads only `speech.wav`, and only after the record log shows a successful `cartesia_speak`.
