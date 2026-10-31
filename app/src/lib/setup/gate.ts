import type { FabricSettingsRead } from '../api/types';
import { setupComplete } from './wizard';

export function shouldShowSetup(read: FabricSettingsRead | null): boolean {
  return !read || !read.exists || !setupComplete(read.settings);
}
