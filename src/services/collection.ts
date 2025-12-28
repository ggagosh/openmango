import type { Db, Document, Filter } from 'mongodb';
import type { CollectionStats, CopyCollectionOptions } from '../types/index.ts';

function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
}

export async function getCollectionStats(db: Db, collName: string): Promise<CollectionStats> {
  const coll = db.collection(collName);
  const stats = await coll.aggregate([{ $collStats: { storageStats: {} } }]).toArray();
  const indexes = await coll.indexes();

  if (stats.length > 0 && stats[0]?.storageStats) {
    const storage = stats[0].storageStats;
    return {
      documentCount: storage.count ?? 0,
      avgDocumentSize: formatBytes(storage.avgObjSize ?? 0),
      totalSize: formatBytes(storage.size ?? 0),
      indexCount: indexes.length,
    };
  }

  // Fallback for older MongoDB or when collStats fails
  const count = await coll.countDocuments();
  return {
    documentCount: count,
    avgDocumentSize: 'N/A',
    totalSize: 'N/A',
    indexCount: indexes.length,
  };
}

export async function createCollection(db: Db, name: string): Promise<void> {
  await db.createCollection(name);
}

export async function dropCollection(db: Db, name: string): Promise<void> {
  await db.dropCollection(name);
}

export async function renameCollection(db: Db, oldName: string, newName: string): Promise<void> {
  await db.renameCollection(oldName, newName);
}

export async function copyCollection(
  sourceDb: Db,
  targetDb: Db,
  sourceName: string,
  options: Omit<CopyCollectionOptions, 'targetConnectionId' | 'targetDatabase'>
): Promise<void> {
  const { dropExisting = false, includeIndexes = true, newName, filterQuery, limit } = options;

  const targetName = newName || sourceName;
  const sourceColl = sourceDb.collection(sourceName);
  const targetColl = targetDb.collection(targetName);

  if (dropExisting) {
    try {
      await targetDb.dropCollection(targetName);
    } catch {
      // Collection might not exist
    }
  }

  // Build filter from query string if provided
  let filter: Filter<Document> = {};
  if (filterQuery) {
    try {
      filter = JSON.parse(filterQuery);
    } catch {
      throw new Error('Invalid filter query JSON');
    }
  }

  // Copy documents
  let cursor = sourceColl.find(filter);
  if (limit && limit > 0) {
    cursor = cursor.limit(limit);
  }

  const allDocs = await cursor.toArray();
  const batchSize = 1000;
  const batches: Document[][] = [];
  for (let i = 0; i < allDocs.length; i += batchSize) {
    batches.push(allDocs.slice(i, i + batchSize));
  }

  await Promise.all(
    batches.filter((batch) => batch.length > 0).map((batch) => targetColl.insertMany(batch))
  );

  // Copy indexes
  if (includeIndexes) {
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
}

export async function countDocuments(db: Db, collName: string, filter?: string): Promise<number> {
  const coll = db.collection(collName);
  let query: Filter<Document> = {};

  if (filter) {
    try {
      query = JSON.parse(filter);
    } catch {
      throw new Error('Invalid filter query JSON');
    }
  }

  return await coll.countDocuments(query);
}

export async function getIndexes(
  db: Db,
  collName: string
): Promise<Array<{ name: string; key: Record<string, unknown>; unique?: boolean }>> {
  const coll = db.collection(collName);
  const indexes = await coll.indexes();
  return indexes.map((idx) => ({
    name: idx.name ?? 'unknown',
    key: idx.key as Record<string, unknown>,
    unique: idx.unique,
  }));
}
