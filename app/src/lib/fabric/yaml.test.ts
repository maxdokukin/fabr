import { describe, expect, it } from 'vitest';
import { parseRecord } from './parse';
import { parseYaml } from './yaml';

const BODY = `## Context
## Spec
A draft.
## Success
The file exists.
## Plan
Write the file.
## Log
`;

describe('block scalars', () => {
  it('reads a stripped literal answer and still exposes the pending approval', () => {
    const text = `---
schema: 6
id: 2026-10-25-write-email
source: null
title: write email
due: null
pri: 1
state: defined
step: approve
waiting: true
paused: false
suspended: false
suspend_until: null
created_at: 2026-10-25T15:38:46Z
updated_at: 2026-10-25T15:39:14Z
requests: {delegate: false, cancel: false, redelegate: false, pause: false, resume: false, complete: false, redo: null}
question: null
answers:
- id: q1
  kind: open
  text: Who is the email for?
  options: null
  prior: null
  fields: null
  asked_at: 2026-10-25T15:38:49Z
  answer:
    value: |-
      Maxim.dokukin@sjsu.edu
      say hi
      Friend
      informal
    replacement: null
    granted: null
    by: gui
    at: 2026-10-25T15:39:04Z
grants:
  requested: [local_write]
  granted: []
plan_meta:
  reversibility: reversible
  tools: [write_file]
  assumptions: []
  checks:
  - cmd: grep
    args:
    - -q
    - Maxim.dokukin@sjsu.edu
    - email_to_maxim.txt
  requires:
    tools: [write_file]
    checks: [grep]
  bound_actions:
  - tool: write_file
    params: '{"content":"To: Maxim.dokukin@sjsu.edu\\nHey Maxim, hi! Hope you''re doing well.","path":"email_to_maxim.txt"}'
    destination: email_to_maxim.txt
    effect: local_write
    grant_class: local_write
assertions:
- id: a1
  text: email_to_maxim.txt contains the recipient address Maxim.dokukin@sjsu.edu.
- id: a2
  text: 'email_to_maxim.txt contains the line Subject: Hi.'
approval:
  status: pending
  by: null
  at: null
  note: null
  digest: abc
verdict: null
cost_usd: 0
links: {redelegated_from: null, redelegated_to: null}
by: {context: agent, spec: agent, success: agent, plan: agent}
needs: null
---
${BODY}`;
    const record = parseRecord(text);
    expect(record.state).toBe('defined');
    expect(record.step).toBe('approve');
    expect(record.approval?.status).toBe('pending');
    expect(record.answers[0].answer?.value).toBe('Maxim.dokukin@sjsu.edu\nsay hi\nFriend\ninformal');
    expect(record.assertions.map((a) => a.text)).toEqual([
      'email_to_maxim.txt contains the recipient address Maxim.dokukin@sjsu.edu.',
      'email_to_maxim.txt contains the line Subject: Hi.',
    ]);
    expect(record.plan_meta?.bound_actions[0].params).toContain("you're");
  });

  it('clips a literal block to one trailing newline and folds a folded block', () => {
    expect(parseYaml('note: |\n  one\n  two\n')).toEqual({ note: 'one\ntwo\n' });
    expect(parseYaml('note: >\n  say hi\n  from a friend\n')).toEqual({ note: 'say hi from a friend\n' });
    expect(parseYaml('note: |-\n  only\n')).toEqual({ note: 'only' });
  });
});
