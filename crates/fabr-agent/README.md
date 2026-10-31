# fabr-agent

This crate moves task records through the pipeline and runs Claude Code for the model steps. It reads and writes the records in each folder and the registry under `~/.fabric-space/`. The desktop app hosts it, and `fabr agent` runs it from the command line.

Open these files first:

- `src/lifecycle.rs` holds the eight steps, their gates and the record writes.
- `src/claude.rs` starts `claude -p` for the planner, the executor and the validator, and reads its stream-json output.
- `src/engine.rs` defines the `Engine` trait and `ScriptedEngine`, the fake used by unit tests.
- `src/runner/mod.rs` is the loop that scans folders, handles requests, writes events and drives the Telegram bot.

Run the tests with `cargo test -p fabr-agent`. They use a fake `claude` script and never call the real CLI. To run one tiny real task with Claude Code, which costs about $0.20, use `FABR_LIVE_CLAUDE=1 cargo test -p fabr-agent --test claude live_ -- --nocapture`.
