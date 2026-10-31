# Telegram message templates

The transport is `crates/fabr-core/src/telegram.rs`. The runner sends the messages below, polls `getUpdates` every 3 seconds, and writes answers and approvals through the same functions the GUI uses, with `by: telegram`. It has been tested against a mock Bot API only. This file is the only copy of the wording for the five message kinds and the button rows. The callbacks write the record fields named under each kind, and the bot ignores any user id other than `telegram_user_id`. Placeholders in {braces} name a record field, an events.log value or the GUI link. Nothing else is used. The app has no deep-link scheme, so `{gui_link}` is the line `Open fabr for details.`

Every message starts with the title line `{title}` and ends with `{gui_link}`, the deep link to
the task in the GUI. A bank value never appears in any message. Messages from a user id other than
`telegram_user_id` are ignored.

Values: `{title}`, `{question.text}`, `{question.options}`, `{question.prior.text}`,
`{question.fields}`, `{plan}` (the `## Plan` section), `{assertions}` (each `text`),
`{grants.requested}`, `{plan_meta.reversibility}`, `{plan_meta.tools}`, `{plan_meta.assumptions}`
(each `text`), `{event.text}` (the free text of the events.log line).

## 1. Question, open

Sent when `question.kind` is open.
```
{title}
Question: {question.text}
Reply to this message with your answer.
{gui_link}
```

Buttons: none. The next text reply is the answer. It goes to `answer.value`.

## 2. Question, choice

Sent when `question.kind` is choice.
```
{title}
{question.text}
Last time: {question.prior.text}
{gui_link}
```

Buttons, one row: `Once` `Always` `No, something else`.

| Button | Writes |
|---|---|
| Once | `answer.value: once` |
| Always | `answer.value: always` |
| No, something else | `answer.value: "no"`, then the follow-up below fills `answer.replacement` |

`No` prompts for `replacement`. A text reply to the choice message counts as `no` with the text as `replacement`, unless the text is exactly once, always or no.

## 3. Grant request

Sent when `question.kind` is grant.
```
{title}
{question.text}
Fields: {question.fields}
{gui_link}
```

Buttons, one row: `Allow for this task` `Review in GUI`.

| Button | Writes |
|---|---|
| Allow for this task | `answer.granted`: one `{field, scope: task}` per entry of `question.fields` |
| Review in GUI | nothing; opens `{gui_link}` |

Allow writes `answer.granted` with scope task.

## 4. Approval

Sent when `approval.status` is pending.
```
{title}
Plan:
{plan}
Checks: {assertions}
Needs: {grants.requested}
Tools: {plan_meta.tools}
Assumed: {plan_meta.assumptions}
Reversible: {plan_meta.reversibility}
{gui_link}
```

When `plan_meta.reversibility` is irreversible, add the line `This cannot be undone.` after
`Reversible:`. If the text passes Telegram's 4096 characters, drop `{plan}`; the GUI shows the full plan.

Buttons, one row: `Approve` `Refuse` `Open in GUI`.

| Button | Writes |
|---|---|
| Approve | `approval.status: approved`, `by: telegram`, `at` |
| Refuse | `approval.status: refused`, `by: telegram`, `at`, and `approval.note` from the follow-up |
| Open in GUI | nothing; opens `{gui_link}` |

Buttons write `approval.*`; `Refuse` prompts for `note`.

## 5. Notice

Sent once for each events.log line. All four kinds have one button, `Open in GUI`, which writes
nothing.

| Event kind | Sent when | Text |
|---|---|---|
| waiting | the agent waits on the user | `{title}` / `Waiting on you: {event.text}` / `{gui_link}` |
| done | `verdict.result` is pass | `{title}` / `Done: {event.text}` / `{gui_link}` |
| failed | `verdict.result` is fail, or cancel | `{title}` / `Failed: {event.text}` / `{gui_link}` |
| stuck | a registry entry is dead | `{title}` / `Stuck: {event.text}` / `{gui_link}` |

## Follow-up prompts

Each follow-up is one plain text message sent after the button tap. The next text reply from
`telegram_user_id` is the answer. No button row.

- After "No, something else": `What should I do instead of: {question.prior.text}?` The reply
  goes to `answer.replacement`; `answer.value` is "no".
- After "Refuse": `Why refuse? Your note goes into the log.` The reply goes to `approval.note`.

Until the reply arrives, `waiting` stays true. The first answer from either channel wins; a late
reply is discarded and the user gets `Already answered in the GUI.`
