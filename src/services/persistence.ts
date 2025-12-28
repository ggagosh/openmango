import { homedir } from 'node:os';
import { join } from 'node:path';
import type { Connection } from '../types/index.ts';

const CONFIG_DIR = join(homedir(), '.openmango');
const CONNECTIONS_FILE = join(CONFIG_DIR, 'connections.json');

// Persisted connection format (never store parsed or status - those are runtime only)
interface PersistedConnection {
  id: string;
  alias: string;
  url: string;
  createdAt: string;
  lastConnectedAt?: string;
}

/**
 * Ensure the config directory exists
 */
async function ensureConfigDir(): Promise<void> {
  const fs = await import('node:fs/promises');
  try {
    await fs.mkdir(CONFIG_DIR, { recursive: true });
  } catch (error) {
    // Directory may already exist
    if ((error as NodeJS.ErrnoException).code !== 'EEXIST') {
      throw error;
    }
  }
}

/**
 * Load connections from disk
 */
export async function loadConnections(): Promise<Connection[]> {
  const fs = await import('node:fs/promises');

  try {
    const data = await fs.readFile(CONNECTIONS_FILE, 'utf-8');
    const persisted: PersistedConnection[] = JSON.parse(data);

    return persisted.map((p) => ({
      id: p.id,
      alias: p.alias,
      url: p.url,
      status: 'disconnected' as const,
      createdAt: p.createdAt,
      lastConnectedAt: p.lastConnectedAt,
    }));
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') {
      return []; // File doesn't exist yet
    }
    throw error;
  }
}

/**
 * Save connections to disk
 */
export async function saveConnections(connections: Connection[]): Promise<void> {
  const fs = await import('node:fs/promises');

  await ensureConfigDir();

  const persisted: PersistedConnection[] = connections.map((c) => ({
    id: c.id,
    alias: c.alias,
    url: c.url,
    createdAt: c.createdAt ?? new Date().toISOString(),
    lastConnectedAt: c.lastConnectedAt,
  }));

  await fs.writeFile(CONNECTIONS_FILE, JSON.stringify(persisted, null, 2), 'utf-8');
}

/**
 * Add a single connection and persist
 */
export async function addConnection(connection: Connection): Promise<void> {
  const existing = await loadConnections();
  await saveConnections([...existing, connection]);
}

/**
 * Remove a connection by ID and persist
 */
export async function removeConnection(id: string): Promise<void> {
  const existing = await loadConnections();
  await saveConnections(existing.filter((c) => c.id !== id));
}

/**
 * Update a connection and persist
 */
export async function updateConnection(id: string, updates: Partial<Connection>): Promise<void> {
  const existing = await loadConnections();
  const updated = existing.map((c) => (c.id === id ? { ...c, ...updates } : c));
  await saveConnections(updated);
}

/**
 * Get the config file path (for display to user)
 */
export function getConfigPath(): string {
  return CONNECTIONS_FILE;
}
