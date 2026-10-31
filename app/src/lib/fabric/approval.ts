import { SCOPES, isoNow } from './answer';
import { parseRecord } from './parse';
import { serializeRecord } from './serialize';
import { updatedAtOf, type FabricSource } from './source';
import type { Scope, TaskRecord, VerdictItem } from './types';
import { ConflictError } from './write';

export interface ApprovalDecision {
  status: 'approved' | 'refused';
  note?: string | null;
  granted?: { field: string; scope?: Scope }[];
}

export function needsHumanApproval(record: TaskRecord, alwaysAsk: string[] = []): boolean {
  if (record.plan_meta?.reversibility === 'irreversible') return true;
  return record.grants.requested.some((f) => f === 'payment' || f.startsWith('payment.') || alwaysAsk.includes(f));
}

export const REPLAN_MARK = 'replanning after failed check';

export function replanFailures(record: TaskRecord): VerdictItem[] | null {
  if (!record.waiting || record.approval?.status !== 'pending') return null;
  if (!record.sections.logEntries.some((e) => e.step === 'evaluate' && e.message.startsWith(REPLAN_MARK))) return null;
  return (record.verdict?.items ?? []).filter((i) => !i.pass);
}

export function verdictIsPrevious(record: TaskRecord): boolean {
  if (record.state === 'succeeded' || record.state === 'failed' || record.verdict === null) return false;
  return record.sections.logEntries.some((e) => e.step === 'evaluate' && e.message.startsWith(REPLAN_MARK));
}

export function applyApproval(record: TaskRecord, decision: ApprovalDecision, by: 'gui' | 'telegram', now: Date = new Date()): TaskRecord {
  if (record.approval === null) throw new Error('this record has no approval to decide');
  if (record.approval.status !== 'pending') throw new Error('this approval is already decided');
  const at = isoNow(now);
  const note = decision.note?.trim() || null;
  const approval = { status: decision.status, by, at, note, digest: record.approval.digest ?? null };
  if (decision.status === 'refused') return { ...record, updated_at: at, approval };
  const seen = new Set<string>();
  const granted = (decision.granted ?? []).map((g) => {
    const scope = g.scope ?? 'task';
    if (!record.grants.requested.includes(g.field)) throw new Error(`field "${g.field}" was not requested`);
    if (!SCOPES.includes(scope)) throw new Error(`scope "${String(scope)}" is not task, project or always`);
    if (seen.has(g.field)) throw new Error(`field "${g.field}" is listed twice`);
    seen.add(g.field);
    return { field: g.field, scope };
  });
  return { ...record, updated_at: at, approval, grants: { ...record.grants, granted } };
}

export async function submitApproval(source: FabricSource, path: string, readText: string, decision: ApprovalDecision, by: 'gui' | 'telegram' = 'gui'): Promise<{ text: string; record: TaskRecord }> {
  const applied = applyApproval(parseRecord(readText), decision, by);
  const expected = updatedAtOf(readText);
  try {
    await source.writeRecord(path, serializeRecord(applied), expected);
  } catch (e) {
    const disk = await source.readRecord(path).catch(() => null);
    if (disk !== null && updatedAtOf(disk) !== expected) throw new ConflictError(path);
    throw e;
  }
  const text = await source.readRecord(path);
  return { text, record: parseRecord(text) };
}
