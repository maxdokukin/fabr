# Driving Claude Code

Facts about the `claude` CLI that the engine depends on and that the code does not explain. The lead checked them against Claude Code 2.1.287 on 2026-10-30.

- `claude -p --output-format stream-json --verbose` first prints a `system`/`init` line with `session_id`. Then it prints one `assistant` line per turn with `tool_use` blocks. The last line is a `result` with `structured_output`, `total_cost_usd`, `is_error`, `subtype` and `permission_denials`.
- `--json-schema` makes Claude Code offer a `StructuredOutput` tool. Small models, Haiku in particular, sometimes answer in prose and claim they already called it. `strict` in `claude.rs` resumes the session once with a request to call it; the executor then falls back to its text. The transcript reader hides `StructuredOutput` entries.
- `--permission-prompts none` turns every prompt into a denial that shows up in `permission_denials`. Without it a headless run blocks.
- In acceptEdits, Claude Code itself allows some Bash commands inside the working folder without a rule, such as `touch` and `date`. Only read-only mode, through `--tools`, is a hard limit.
- The approval digest is a plain SHA-256, not a keyed MAC, and Claude Code runs as the user. An approved Bash rule could therefore rewrite the record and its digest. The real fence is that Bash runs only exact, approved commands; the edit tools are denied on `.fabric-space/**`.
- The user's own `~/.claude` settings, hooks and allow rules apply to every session fabr starts. There is no isolation flag in use.
- `--name` writes `custom-title` and `agent-name` lines into the session file. `ccsession.rs` prefers `custom-title`, then `ai-title`, then the first prompt.
- Session files live in `~/.claude/projects/<cwd>/`, where the cwd has every non-alphanumeric character replaced by `-`. They follow the real home or `CLAUDE_CONFIG_DIR`, never `FABR_HOME`; reading them from `FABR_HOME` was a bug found in the live test.
- `claude auth status` prints JSON with `loggedIn`, `authMethod` and `subscriptionType` and costs nothing. It also prints the email, which setup must not show.
- The app's chat box (`fabric_cc_send`, `ccsession::send`) resumes a session with `claude -p --resume <id> --output-format json` in plan mode with Read, Glob and Grep. It refuses while the owning task runs a step (step set, not waiting, not paused), because two processes on one session would interleave. Its cost is added to the owning record.
- A subtype containing `budget` means `--max-budget-usd` stopped the run.
- Claude Code runs in its own process group (`process_group(0)`), so a kill reaches the tools it started. The group is signalled with TERM, then KILL after 2 seconds.

## Tests

- Tests never call the real binary. `crates/fabr-agent/tests/common/mod.rs` writes a fake `claude` shell script that records argv, cwd and stdin per call and plays canned stream-json, with `@SLEEP`, `@SH` and `@ERR` lines.
- `tests/abort.rs` is its own test binary because `abort_all` sets a process-wide flag.
- The live test runs a tiny real task with Haiku for about $0.20: `FABR_LIVE_CLAUDE=1 cargo test -p fabr-agent --test claude live_ -- --nocapture`.
- A manual end-to-end run: put a `settings.toml` with `default_folder`, `model = "haiku"` and `completed_steps = ["folder", "claude"]` under a temp `FABR_HOME`. Then run `fabr init <folder>`, `fabr add <folder> "<title>"` and `fabr delegate <id prefix>`, and start `FABR_HOME=<home> fabr agent --interval 2 --verbose`. Follow it with `fabr status`, and answer or approve with `fabr answer` and `fabr approve`. Each run costs about $0.15 with Haiku.
