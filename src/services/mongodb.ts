import { MongoClient, type Db, type ListDatabasesResult } from 'mongodb';
import type { TestConnectionResult } from '../types/index.ts';

export interface DatabaseInfo {
  name: string;
  sizeOnDisk: number;
  empty: boolean;
}

class MongoDBService {
  private clients: Map<string, MongoClient> = new Map();

  async connect(id: string, url: string): Promise<MongoClient> {
    if (this.clients.has(id)) {
      const existing = this.clients.get(id)!;
      try {
        await existing.db('admin').command({ ping: 1 });
        return existing;
      } catch {
        this.clients.delete(id);
      }
    }

    const client = new MongoClient(url, {
      serverSelectionTimeoutMS: 5000,
      connectTimeoutMS: 5000,
    });

    await client.connect();
    this.clients.set(id, client);
    return client;
  }

  async disconnect(id: string): Promise<void> {
    const client = this.clients.get(id);
    if (client) {
      await client.close();
      this.clients.delete(id);
    }
  }

  async disconnectAll(): Promise<void> {
    const promises = Array.from(this.clients.entries()).map(async ([id]) => {
      await this.disconnect(id);
    });
    await Promise.all(promises);
  }

  getClient(id: string): MongoClient | undefined {
    return this.clients.get(id);
  }

  getDb(connectionId: string, dbName: string): Db | undefined {
    const client = this.clients.get(connectionId);
    return client?.db(dbName);
  }

  isConnected(id: string): boolean {
    return this.clients.has(id);
  }

  async listDatabases(id: string): Promise<DatabaseInfo[]> {
    const client = this.clients.get(id);
    if (!client) {
      throw new Error(`No connection found for id: ${id}`);
    }

    const result: ListDatabasesResult = await client.db('admin').admin().listDatabases();
    return result.databases.map((db) => ({
      name: db.name,
      sizeOnDisk: db.sizeOnDisk ?? 0,
      empty: db.empty ?? false,
    }));
  }

  async ping(id: string): Promise<boolean> {
    const client = this.clients.get(id);
    if (!client) return false;

    try {
      await client.db('admin').command({ ping: 1 });
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Test a connection without persisting it
   * Returns detailed error information for the UI
   */
  async testConnection(url: string): Promise<TestConnectionResult> {
    const startTime = Date.now();
    const tempClient = new MongoClient(url, {
      serverSelectionTimeoutMS: 5000,
      connectTimeoutMS: 5000,
    });

    try {
      await tempClient.connect();
      const adminDb = tempClient.db('admin');

      // Run a ping to verify connection
      await adminDb.command({ ping: 1 });

      // Get server info if possible
      let serverInfo: TestConnectionResult['serverInfo'];
      try {
        const buildInfo = await adminDb.command({ buildInfo: 1 });
        let replSetStatus: { set?: string } | null = null;
        try {
          replSetStatus = await adminDb.command({ replSetGetStatus: 1 });
        } catch {
          // Not a replica set, or no permissions
        }

        serverInfo = {
          version: buildInfo.version,
          isReplicaSet: !!replSetStatus,
          replicaSetName: replSetStatus?.set,
        };
      } catch {
        // Server info is optional
      }

      const latencyMs = Date.now() - startTime;

      return {
        success: true,
        latencyMs,
        serverInfo,
      };
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Unknown error';

      // Categorize the error type
      let errorType: TestConnectionResult['errorType'] = 'unknown';

      if (message.includes('ECONNREFUSED') || message.includes('ENOTFOUND')) {
        errorType = 'network';
      } else if (message.includes('Authentication failed') || message.includes('auth')) {
        errorType = 'auth';
      } else if (message.includes('timed out') || message.includes('timeout')) {
        errorType = 'timeout';
      } else if (message.includes('querySrv') || message.includes('DNS')) {
        errorType = 'dns';
      } else if (
        message.includes('SSL') ||
        message.includes('TLS') ||
        message.includes('certificate')
      ) {
        errorType = 'ssl';
      }

      return {
        success: false,
        error: message,
        errorType,
      };
    } finally {
      await tempClient.close().catch(() => {});
    }
  }
}

export const mongoService = new MongoDBService();
