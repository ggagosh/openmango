import type { MongoClient } from 'mongodb';
import type { DatabaseStats } from '../types/index.ts';

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
}

export async function getDatabaseStats(
  client: MongoClient,
  dbName: string
): Promise<DatabaseStats> {
  const db = client.db(dbName);
  const stats = await db.stats();

  return {
    sizeOnDisk: formatBytes(stats.dataSize + stats.indexSize),
    collectionCount: stats.collections,
    documentCount: stats.objects,
  };
}

export async function listCollections(client: MongoClient, dbName: string): Promise<string[]> {
  const db = client.db(dbName);
  const collections = await db.listCollections().toArray();
  return collections.map((c) => c.name).sort();
}

export async function createDatabase(client: MongoClient, dbName: string): Promise<void> {
  const db = client.db(dbName);
  await db.createCollection('_init');
  await db.dropCollection('_init');
}

export async function dropDatabase(client: MongoClient, dbName: string): Promise<void> {
  await client.db(dbName).dropDatabase();
}

async function copyCollectionData(
  sourceColl: ReturnType<typeof sourceDb.collection>,
  targetColl: ReturnType<typeof targetDb.collection>,
  batchSize = 1000
): Promise<void> {
  const cursor = sourceColl.find({});
  const allDocs = await cursor.toArray();

  const batches: (typeof allDocs)[] = [];
  for (let i = 0; i < allDocs.length; i += batchSize) {
    batches.push(allDocs.slice(i, i + batchSize));
  }

  await Promise.all(
    batches.filter((batch) => batch.length > 0).map((batch) => targetColl.insertMany(batch))
  );
}

async function copyCollectionIndexes(
  sourceColl: ReturnType<typeof sourceDb.collection>,
  targetColl: ReturnType<typeof targetDb.collection>
): Promise<void> {
  const indexes = await sourceColl.indexes();
  const indexPromises = indexes
    .filter((index) => index.name !== '_id_')
    .map(async (index) => {
      try {
        await targetColl.createIndex(index.key, {
          name: index.name,
          unique: index.unique,
          sparse: index.sparse,
          background: true,
        });
      } catch {
        // Skip indexes that fail
      }
    });
  await Promise.all(indexPromises);
}

// Helper type for collection reference
declare const sourceDb: ReturnType<MongoClient['db']>;
declare const targetDb: ReturnType<MongoClient['db']>;

export async function copyDatabase(
  sourceClient: MongoClient,
  targetClient: MongoClient,
  sourceDbName: string,
  targetDbName: string,
  options: {
    dropExisting?: boolean;
    includeIndexes?: boolean;
    selectedCollections?: string[];
  } = {}
): Promise<void> {
  const { dropExisting = false, includeIndexes = true, selectedCollections } = options;

  const sourceDatabase = sourceClient.db(sourceDbName);
  const targetDatabase = targetClient.db(targetDbName);

  if (dropExisting) {
    await targetDatabase.dropDatabase();
  }

  const collections = await sourceDatabase.listCollections().toArray();
  const collectionsToCopy = selectedCollections
    ? collections.filter((c) => selectedCollections.includes(c.name))
    : collections;

  const copyPromises = collectionsToCopy
    .filter((collInfo) => collInfo.type !== 'view')
    .map(async (collInfo) => {
      const sourceColl = sourceDatabase.collection(collInfo.name);
      const targetColl = targetDatabase.collection(collInfo.name);

      await copyCollectionData(sourceColl, targetColl);

      if (includeIndexes) {
        await copyCollectionIndexes(sourceColl, targetColl);
      }
    });

  await Promise.all(copyPromises);
}
