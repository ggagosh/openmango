import type { Document, ObjectId } from 'mongodb';

function isObjectId(value: unknown): value is ObjectId {
  return (
    value !== null &&
    typeof value === 'object' &&
    '_bsontype' in value &&
    value._bsontype === 'ObjectId'
  );
}

function isDate(value: unknown): value is Date {
  return value instanceof Date;
}

function formatValue(value: unknown, indent: string, currentIndent: string): string {
  if (value === null) {
    return 'null';
  }

  if (value === undefined) {
    return 'undefined';
  }

  if (typeof value === 'string') {
    const escaped = value
      .replace(/\\/g, '\\\\')
      .replace(/"/g, '\\"')
      .replace(/\n/g, '\\n')
      .replace(/\r/g, '\\r')
      .replace(/\t/g, '\\t');
    return `"${escaped}"`;
  }

  if (typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }

  if (isObjectId(value)) {
    return `ObjectId("${value.toString()}")`;
  }

  if (isDate(value)) {
    return `ISODate("${value.toISOString()}")`;
  }

  if (Array.isArray(value)) {
    if (value.length === 0) {
      return '[]';
    }

    const nextIndent = currentIndent + indent;
    const items = value.map((item) => {
      return `${nextIndent}${formatValue(item, indent, nextIndent)}`;
    });

    return `[\n${items.join(',\n')}\n${currentIndent}]`;
  }

  if (typeof value === 'object') {
    const obj = value as Record<string, unknown>;
    const keys = Object.keys(obj);

    if (keys.length === 0) {
      return '{}';
    }

    const nextIndent = currentIndent + indent;
    const entries = keys.map((key) => {
      const formattedValue = formatValue(obj[key], indent, nextIndent);
      return `${nextIndent}"${key}": ${formattedValue}`;
    });

    return `{\n${entries.join(',\n')}\n${currentIndent}}`;
  }

  // Unknown type - try to serialize or return placeholder
  try {
    return JSON.stringify(value);
  } catch {
    return '"[unserializable]"';
  }
}

export function formatDocument(doc: Document, indentSize: number = 2): string {
  const indent = ' '.repeat(indentSize);
  return formatValue(doc, indent, '');
}

export function parseEditedDocument(jsonString: string): Document {
  let parsed = jsonString
    .replace(/ObjectId\("([^"]+)"\)/g, '"$1"')
    .replace(/ISODate\("([^"]+)"\)/g, '"$1"');

  try {
    return JSON.parse(parsed);
  } catch {
    throw new Error('Invalid JSON syntax');
  }
}

export function validateJson(jsonString: string): { valid: boolean; error?: string } {
  try {
    parseEditedDocument(jsonString);
    return { valid: true };
  } catch (e) {
    return {
      valid: false,
      error: e instanceof Error ? e.message : 'Invalid JSON',
    };
  }
}
