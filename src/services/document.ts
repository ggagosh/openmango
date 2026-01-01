import type { Db, Document, Filter, ObjectId, WithId } from 'mongodb';
import type { FindDocumentsOptions, FindDocumentsResult } from '../types/index.ts';

export async function findDocuments(
  db: Db,
  collectionName: string,
  options: FindDocumentsOptions = {}
): Promise<FindDocumentsResult> {
  const { filter, projection, sort, limit = 25, skip = 0 } = options;
  const collection = db.collection(collectionName);

  let query: Filter<Document> = {};
  if (filter) {
    try {
      query = JSON.parse(filter);
    } catch {
      throw new Error('Invalid filter query JSON');
    }
  }

  const [documents, totalCount] = await Promise.all([
    collection
      .find(query, { projection })
      .sort(sort ?? {})
      .skip(skip)
      .limit(limit)
      .toArray(),
    collection.countDocuments(query),
  ]);

  return { documents, totalCount };
}

export async function getDocument(
  db: Db,
  collectionName: string,
  id: ObjectId
): Promise<WithId<Document> | null> {
  const collection = db.collection(collectionName);
  return collection.findOne({ _id: id });
}

export async function insertDocument(
  db: Db,
  collectionName: string,
  document: Document
): Promise<ObjectId> {
  const collection = db.collection(collectionName);
  const result = await collection.insertOne(document);
  return result.insertedId;
}

export async function updateDocument(
  db: Db,
  collectionName: string,
  id: ObjectId,
  document: Document
): Promise<boolean> {
  const collection = db.collection(collectionName);
  const { _id, ...updateData } = document;
  const result = await collection.replaceOne({ _id: id }, updateData);
  return result.modifiedCount > 0;
}

export async function deleteDocument(
  db: Db,
  collectionName: string,
  id: ObjectId
): Promise<boolean> {
  const collection = db.collection(collectionName);
  const result = await collection.deleteOne({ _id: id });
  return result.deletedCount > 0;
}

export async function duplicateDocument(
  db: Db,
  collectionName: string,
  id: ObjectId
): Promise<ObjectId> {
  const collection = db.collection(collectionName);
  const original = await collection.findOne({ _id: id });

  if (!original) {
    throw new Error('Document not found');
  }

  const { _id, ...docWithoutId } = original;
  const result = await collection.insertOne(docWithoutId);
  return result.insertedId;
}
