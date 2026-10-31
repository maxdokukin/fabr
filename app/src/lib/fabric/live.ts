import { createApi } from '../api/index';
import { isTauri } from '../api/env';
import type { FabricSettingsRead } from '../api/types';
import { FixtureSource, type FabricSource, type RawRecord } from './source';
import { TauriSource } from './tauri-source';

export const FOLDERS_UNKNOWN_USER = 'Task folders are not set up yet. Finish setup to choose your default folder.';

export interface LiveLocations { roots: string[]; home: string }

export function pickLocations(env: { root?: string; home?: string }, homeDir: string | null, settings?: FabricSettingsRead | null): LiveLocations {
  const folder = settings?.exists ? settings.settings.default_folder.replace(/\/+$/, '') : '';
  const root = env.root || folder;
  const home = env.home || homeDir || folder;
  if (!home || !root) {
    if (import.meta.env.DEV) console.warn('fabric folders unknown: finish setup, or set VITE_FABRIC_ROOT and VITE_FABRIC_HOME');
    throw new Error(FOLDERS_UNKNOWN_USER);
  }
  return { roots: [root], home };
}

class MockFixtures extends FixtureSource {
  async listRegistry(): Promise<RawRecord[]> {
    const at = new Date().toISOString();
    return (await super.listRegistry()).map((r) => (r.text.includes('rename-scans') ? r : { ...r, text: r.text.replace(/^heartbeat_at:.*$/m, `heartbeat_at: ${at}`) }));
  }
  async readBackendHeartbeat(): Promise<string | null> {
    return new Date().toISOString();
  }
  async stamp(): Promise<string> {
    return `${await super.stamp()}:${Math.floor(Date.now() / 5000)}`;
  }
}

export async function createFabricSource(settings?: FabricSettingsRead | null, treeRoots: string[] = []): Promise<FabricSource> {
  if (!isTauri() && import.meta.env.VITE_MOCK === '1') return new MockFixtures();
  let homeDir: string | null = null;
  if (isTauri()) homeDir = (await (await import('@tauri-apps/api/path')).homeDir()).replace(/\/+$/, '');
  const api = await createApi();
  if (!isTauri()) homeDir = await api.fabricHome().then((h) => h.replace(/\/+$/, ''), () => null);
  const read = settings ?? (await api.fabricSettingsRead());
  const { roots, home } = pickLocations({ root: import.meta.env.VITE_FABRIC_ROOT, home: import.meta.env.VITE_FABRIC_HOME }, homeDir, read);
  return new TauriSource(api, treeRoots.length ? treeRoots : roots, home);
}
