import { FIELDS, SCHEMA, SECTION_HEADINGS, type Answer, type Question, type TaskRecord } from './types';

const RESERVED = new Set(['null', '~', 'true', 'false', 'yes', 'no', 'on', 'off']);
const BARE = /^[A-Za-z0-9_./][\w./:+-]*$/;

function str(s: string): string {
  if (BARE.test(s) && !RESERVED.has(s.toLowerCase()) && !/^-?\d+(\.\d+)?$/.test(s) && !s.endsWith(':')) return s;
  return `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n')}"`;
}

function scalar(v: unknown): string {
  if (v === null || v === undefined) return 'null';
  if (typeof v === 'string') return str(v);
  return String(v);
}

function flowList(xs: unknown[]): string {
  return `[${xs.map(scalar).join(', ')}]`;
}

function flowMap(o: object): string {
  const parts = Object.entries(o).map(([k, v]) => `${k}: ${scalar(v)}`);
  return `{${parts.join(', ')}}`;
}

function strList(xs: string[] | null): string {
  return xs === null ? 'null' : flowList(xs);
}

function pad(n: number): string {
  return ' '.repeat(n);
}

function answerLines(a: Answer, ind: number): string[] {
  const g = a.granted === null ? null : a.granted;
  const out = [`${pad(ind)}value: ${scalar(a.value)}`, `${pad(ind)}replacement: ${scalar(a.replacement)}`];
  if (g === null) out.push(`${pad(ind)}granted: null`);
  else if (!g.length) out.push(`${pad(ind)}granted: []`);
  else {
    out.push(`${pad(ind)}granted:`);
    for (const x of g) out.push(`${pad(ind + 2)}- field: ${scalar(x.field)}`, `${pad(ind + 4)}scope: ${scalar(x.scope)}`);
  }
  out.push(`${pad(ind)}by: ${scalar(a.by)}`, `${pad(ind)}at: ${scalar(a.at)}`);
  return out;
}

function questionLines(q: Question, ind: number): string[] {
  const out = [
    `${pad(ind)}id: ${scalar(q.id)}`,
    `${pad(ind)}kind: ${scalar(q.kind)}`,
    `${pad(ind)}text: ${scalar(q.text)}`,
    `${pad(ind)}options: ${strList(q.options)}`,
  ];
  if (q.prior === null) out.push(`${pad(ind)}prior: null`);
  else {
    out.push(`${pad(ind)}prior:`);
    out.push(`${pad(ind + 2)}text: ${scalar(q.prior.text)}`, `${pad(ind + 2)}source_task: ${scalar(q.prior.source_task)}`, `${pad(ind + 2)}last_confirmed: ${scalar(q.prior.last_confirmed)}`);
  }
  out.push(`${pad(ind)}fields: ${strList(q.fields)}`, `${pad(ind)}asked_at: ${scalar(q.asked_at)}`);
  if (q.answer === null) out.push(`${pad(ind)}answer: null`);
  else out.push(`${pad(ind)}answer:`, ...answerLines(q.answer, ind + 2));
  return out;
}

function itemList(key: string, items: object[]): string[] {
  if (!items.length) return [`${key}: []`];
  const out = [`${key}:`];
  for (const it of items) {
    Object.entries(it).forEach(([k, v], i) => out.push(`${i === 0 ? '  - ' : '    '}${k}: ${scalar(v)}`));
  }
  return out;
}

export function serializeFrontMatter(r: TaskRecord): string[] {
  const out: string[] = [];
  const add = (...l: string[]) => out.push(...l);
  for (const f of FIELDS) {
    switch (f) {
      case 'schema': add(`schema: ${SCHEMA}`); break;
      case 'requests': {
        const q = r.requests;
        add(`requests: ${flowMap({ delegate: q.delegate, cancel: q.cancel, redelegate: q.redelegate, pause: !!q.pause, resume: !!q.resume, complete: !!q.complete, redo: q.redo ?? null })}`);
        break;
      }
      case 'by': add(`by: ${flowMap({ context: r.by.context, spec: r.by.spec, success: r.by.success, plan: r.by.plan })}`); break;
      case 'links': add(`links: ${flowMap(r.links)}`); break;
      case 'source':
        if (r.source === null) add('source: null');
        else add(`source: ${flowMap(r.source)}`);
        break;
      case 'question':
        if (r.question === null) add('question: null');
        else add('question:', ...questionLines(r.question, 2));
        break;
      case 'answers':
        if (!r.answers.length) add('answers: []');
        else {
          add('answers:');
          for (const q of r.answers) {
            const ls = questionLines(q, 4);
            ls[0] = `  - ${ls[0].slice(4)}`;
            add(...ls);
          }
        }
        break;
      case 'grants': {
        add('grants:', `  requested: ${flowList(r.grants.requested)}`);
        if (!r.grants.granted.length) add('  granted: []');
        else {
          add('  granted:');
          for (const g of r.grants.granted) add(`    - field: ${scalar(g.field)}`, `      scope: ${scalar(g.scope)}`);
        }
        break;
      }
      case 'plan_meta': {
        const p = r.plan_meta;
        if (p === null) { add('plan_meta: null'); break; }
        add('plan_meta:', `  reversibility: ${scalar(p.reversibility)}`, `  tools: ${flowList(p.tools)}`, '  requires:', `    tools: ${flowList(p.requires.tools)}`, `    checks: ${flowList(p.requires.checks)}`);
        if (!p.assumptions.length) add('  assumptions: []');
        else {
          add('  assumptions:');
          for (const a of p.assumptions) add(`    - rule_id: ${scalar(a.rule_id)}`, `      text: ${scalar(a.text)}`);
        }
        if (Array.isArray(p.checks)) {
          if (!p.checks.length) add('  checks: []');
          else {
            add('  checks:');
            for (const c of p.checks) add(`    - cmd: ${scalar(c.cmd)}`, `      args: ${flowList(c.args)}`);
          }
        }
        if (p.bound_actions?.length) {
          add('  bound_actions:');
          for (const a of p.bound_actions) {
            add(
              `    - tool: ${scalar(a.tool)}`,
              `      params: ${scalar(a.params)}`,
              `      destination: ${scalar(a.destination)}`,
              `      effect: ${scalar(a.effect)}`,
              `      grant_class: ${scalar(a.grant_class)}`,
            );
          }
        }
        break;
      }
      case 'assertions': add(...itemList('assertions', r.assertions)); break;
      case 'approval': {
        const a = r.approval;
        if (a === null) add('approval: null');
        else add(`approval: ${flowMap(Object.fromEntries((['status', 'by', 'at', 'note', 'digest'] as const).filter((k) => k in a && a[k] !== undefined).map((k) => [k, a[k]])))}`);
        break;
      }
      case 'verdict': {
        const v = r.verdict;
        if (v === null) { add('verdict: null'); break; }
        add('verdict:', `  result: ${scalar(v.result)}`, `  at: ${scalar(v.at)}`);
        if (!v.items.length) add('  items: []');
        else {
          add('  items:');
          for (const i of v.items) add(`    - id: ${scalar(i.id)}`, `      pass: ${scalar(i.pass)}`, `      evidence: ${scalar(i.evidence)}`);
        }
        break;
      }
      default: add(`${f}: ${scalar(r[f])}`);
    }
  }
  return out;
}

export function serializeRecord(r: TaskRecord): string {
  const s = r.sections;
  const texts = [s.context, s.spec, s.success, s.plan, s.log];
  const body = SECTION_HEADINGS.flatMap((h, i) => [`## ${h}`, ...(texts[i] ? [texts[i]] : [])]);
  return ['---', ...serializeFrontMatter(r), '---', ...body, ''].join('\n');
}
