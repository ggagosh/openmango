import type { Db } from 'mongodb';

interface FieldIndexEntry {
  fields: string[];
  timestamp: number;
}

// In-memory cache with TTL
const fieldCache = new Map<string, FieldIndexEntry>();
const CACHE_TTL = 10 * 60 * 1000; // 10 minutes

/**
 * Extract all field paths from an object recursively.
 * Handles nested objects up to maxDepth.
 */
function extractFieldPaths(
  obj: unknown,
  prefix: string,
  fields: Set<string>,
  maxDepth: number
): void {
  if (maxDepth <= 0 || !obj || typeof obj !== 'object') return;
  if (Array.isArray(obj)) return; // Skip array indices

  // Skip BSON types
  if ('_bsontype' in (obj as object)) return;

  for (const [key, value] of Object.entries(obj)) {
    const path = prefix ? `${prefix}.${key}` : key;
    fields.add(path);
    extractFieldPaths(value, path, fields, maxDepth - 1);
  }
}

/**
 * Get all field names from a collection by sampling documents.
 * Results are cached for performance.
 */
export async function getCollectionFields(
  db: Db,
  collectionName: string,
  sampleSize = 100
): Promise<string[]> {
  const cacheKey = `${db.databaseName}:${collectionName}`;

  // Check cache
  const cached = fieldCache.get(cacheKey);
  if (cached && Date.now() - cached.timestamp < CACHE_TTL) {
    return cached.fields;
  }

  // Sample documents
  const docs = await db.collection(collectionName).find({}).limit(sampleSize).toArray();

  // Extract all field paths
  const fields = new Set<string>();
  for (const doc of docs) {
    extractFieldPaths(doc, '', fields, 5); // max depth 5
  }

  const fieldList = Array.from(fields).sort();

  // Cache result
  fieldCache.set(cacheKey, {
    fields: fieldList,
    timestamp: Date.now(),
  });

  return fieldList;
}

/**
 * Clear cached fields for a collection.
 */
export function clearFieldCache(dbName: string, collectionName: string): void {
  fieldCache.delete(`${dbName}:${collectionName}`);
}

/**
 * Clear all cached fields.
 */
export function clearAllFieldCache(): void {
  fieldCache.clear();
}

/**
 * MongoDB operators for filter suggestions.
 */
export const MONGO_OPERATORS = [
  { name: '$eq', desc: 'Equals' },
  { name: '$ne', desc: 'Not equals' },
  { name: '$gt', desc: 'Greater than' },
  { name: '$gte', desc: 'Greater or equal' },
  { name: '$lt', desc: 'Less than' },
  { name: '$lte', desc: 'Less or equal' },
  { name: '$in', desc: 'In array' },
  { name: '$nin', desc: 'Not in array' },
  { name: '$regex', desc: 'Regex match' },
  { name: '$exists', desc: 'Field exists' },
] as const;

/**
 * Get suggestions based on current filter input.
 * Returns field names or operators depending on context.
 */
export function getFilterSuggestions(
  input: string,
  fields: string[],
  cursorPosition: number
): Array<{ name: string; desc?: string }> {
  const beforeCursor = input.slice(0, cursorPosition);

  // Check if we're after a colon (suggesting operators) or after a quote (suggesting fields)
  const lastColon = beforeCursor.lastIndexOf(':');
  const lastQuote = beforeCursor.lastIndexOf('"');
  const lastOpenBrace = beforeCursor.lastIndexOf('{');

  // After opening brace or comma - suggest field names
  if (lastOpenBrace > lastColon && lastOpenBrace > lastQuote) {
    const partial = beforeCursor
      .slice(lastOpenBrace + 1)
      .trim()
      .replace(/^"/, '');
    return fields
      .filter((f) => f.toLowerCase().startsWith(partial.toLowerCase()))
      .slice(0, 10)
      .map((f) => ({ name: f }));
  }

  // After colon - check context
  if (lastColon > lastQuote) {
    const afterColon = beforeCursor.slice(lastColon + 1).trim();
    // Only suggest operators if typing { after colon
    if (afterColon.startsWith('{')) {
      const partial = afterColon.slice(1).trim().replace(/^"/, '');
      return MONGO_OPERATORS.filter((op) =>
        op.name.toLowerCase().startsWith(partial.toLowerCase() || '$')
      ).map((op) => ({ name: op.name, desc: op.desc }));
    }
    // After colon without { - entering value, no suggestions
    return [];
  }

  // Default: suggest fields (e.g., after comma)
  const partial = beforeCursor.split(/[{,]/).pop()?.trim().replace(/^"/, '') ?? '';
  if (!partial) return [];
  return fields
    .filter((f) => f.toLowerCase().includes(partial.toLowerCase()))
    .slice(0, 10)
    .map((f) => ({ name: f }));
}
