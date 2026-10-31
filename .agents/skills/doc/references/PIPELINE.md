# The pipeline

What happens after the user presses "delegate". The order is code (`crates/fabr-agent/src/lifecycle.rs`, `advance`). Claude Code works inside a step and never chooses the next one. Two steps are gates where the flow waits for the user: define (a question) and approve. The execute step can also park on a question.

Delegate itself is not a step. It sets `requests.delegate`, and the orchestrator moves the record from reminder to task and sets `step: gather`. The record's `step` field has eight values, listed below.

| Step | Gate | Actor | Reads | Writes |
|---|---|---|---|---|
| gather | no | orchestrator, no model | the folder chain up to the default folder | `### Gathered` in Context: rules, knowledge, learned answers, the node's files |
| define | yes | planner session, new, `sessions.plan` | Context with answers | spec, success, 1 to 3 assertions, or one question |
| plan | no | the same planner session, resumed | spec, assertions | plan with `### Inputs`, `### Outputs`, `### Steps`; `plan_meta` (reversibility, tools, checks); approval digest |
| approve | yes | the user, in the GUI or on Telegram | the defined task | `approval`, `grants.granted` |
| execute | no | executor session, `sessions.execute` | plan, answers | files inside the node, `execute:` log lines per tool use |
| evaluate | no | allowlisted checks, then a validator session, new, `sessions.evaluate` | assertions, the executor's summary, check outcomes | verdict, state succeeded or failed |
| learn | no | orchestrator | answers | `.fabric-space/.agents/learned.yaml` in the node |
| notify | no | orchestrator | the record | `events.log` line, Telegram notice, step cleared |

## How each session runs

`crates/fabr-agent/src/claude.rs` builds the command line. Every call is `claude -p` with stream-json output, a JSON schema, `--session-id <new>` or `--resume <id>`, and `--name "fabr <role>: <title>"`. The model, effort and spend cap come from settings when they are set, and the working directory is the node folder. The prompt goes in on stdin.

| Role | Permission flags |
|---|---|
| planner, validator | `--permission-mode plan --permission-prompts none --tools Read,Glob,Grep` |
| executor | `--permission-mode acceptEdits --permission-prompts none --allowedTools "Read Glob Grep <plan_meta.tools>" --disallowedTools` with Edit, Write, MultiEdit and NotebookEdit on `.fabric-space/**` |
| executor, read-only mode | adds `--tools Read,Glob,Grep` |

## Rules the code enforces

- A second approved task in a node waits (`other_running`) and shows registry `activity: queued`. One in-progress task per node.
- Approval is checked before every executor run, including resumes and restarts: status approved, a digest equal to the recomputed one, a valid plan, and `by` not `auto` when the plan is irreversible. A mismatch sends the task back to approve with a pending approval.
- A session id is passed to `--resume` only when it is a UUID; anything else starts a new session.
- `plan_meta.tools` holds Claude Code permission rules: Read, Glob, Grep, Edit, Write, MultiEdit, NotebookEdit, WebFetch, WebSearch, bare or with a spec, and `Bash(<pattern>)`. Bare `Bash`, `Bash(*)` and unknown names are refused (`validate_plan`).
- A plan without the three subsections gets one repair round on the resumed planner session, then a stage error.
- When the executor hits permission denials and does not finish, the denials become an open question that names the exact rule, such as `Bash(npm install left-pad)`. Only the reply `allow` (or `yes, allow`) adds those rules to `plan_meta.tools`, re-signs the digest and resumes the executor session. Any other answer resumes it without new rules. Wrapper and shell commands (`sh -c`, `env`, `sudo`, `python -c`, pipes, `;`) and paths under `.fabric-space` are never offered. The same denial three times in a row is a stage error.
- Claude Code itself lets read-only commands such as `uname` or `date` run in acceptEdits without a rule, so they never reach the question.
- A run without structured output is resumed once with a request to return it. If the executor still returns only text, that text becomes its summary and evaluate runs. The other roles count a stage error.
- When the user refuses, the record goes back through plan on the same planner session, with the user's note.
- A stage that errors logs `error N`. The third consecutive error on the same stage moves the record to failed. A run that hits the spend cap fails the task at once, because a retry would spend the cap again. With `max_task_usd` set, a task whose `cost_usd` has reached it stops before its next run.
- Cancel, pause, complete and suspend kill a running Claude Code process group within about a second. Quitting the desktop app kills every live run (`claude::abort_all`). Every run's process group is listed in `~/.fabric-space/registry/_runs.txt`, and a runner that starts kills the groups that a dead runner left behind. On Linux the child also gets SIGTERM when fabr dies.
- A run that would resume a session waits while the app's chat holds `<node>/.fabric-space/chat/<session>.lock` with a live pid.
- Cancel applies only to in_progress; in other states the request is cleared and logged as ignored. A cancelled task is failed with `verdict` null and can be redelegated.

## Restart and resume

Every step writes its artifact and the session id to the record before the next one starts. On restart the runner finds records that are in task, defined or in progress with a `step` set, and continues from that step. The executor resumes its own session, so the work it did before the restart is in its context. A record parked on a question or an approval keeps waiting.
