import type { HighlightedSegment } from '../types/index.ts';
import { colors } from '../theme/index.ts';

const jsonColors = {
  key: colors.foreground,
  string: '#4ade80',
  number: '#22d3ee',
  boolean: '#fbbf24',
  null: '#c084fc',
  bracket: colors.muted,
  colon: colors.muted,
  comma: colors.muted,
  objectId: colors.dim,
  date: colors.dim,
};

export function highlightJson(jsonString: string): HighlightedSegment[] {
  const segments: HighlightedSegment[] = [];
  let i = 0;

  const pushSegment = (text: string, color: string) => {
    if (text) {
      segments.push({ text, color });
    }
  };

  const skipWhitespace = (): string => {
    let ws = '';
    while (i < jsonString.length && /\s/.test(jsonString[i]!)) {
      ws += jsonString[i];
      i++;
    }
    return ws;
  };

  const parseString = (): string => {
    let str = '"';
    i++;
    while (i < jsonString.length && jsonString[i] !== '"') {
      if (jsonString[i] === '\\' && i + 1 < jsonString.length) {
        str += jsonString[i]! + jsonString[i + 1]!;
        i += 2;
      } else {
        str += jsonString[i];
        i++;
      }
    }
    if (i < jsonString.length) {
      str += '"';
      i++;
    }
    return str;
  };

  const parseNumber = (): string => {
    let num = '';
    while (i < jsonString.length && /[-+\d.eE]/.test(jsonString[i]!)) {
      num += jsonString[i];
      i++;
    }
    return num;
  };

  const parseObjectId = (): string => {
    let result = 'ObjectId(';
    i += 9;
    const ws1 = skipWhitespace();
    result += ws1;
    if (jsonString[i] === '"') {
      result += parseString();
    }
    const ws2 = skipWhitespace();
    result += ws2;
    if (jsonString[i] === ')') {
      result += ')';
      i++;
    }
    return result;
  };

  const parseISODate = (): string => {
    let result = 'ISODate(';
    i += 8;
    const ws1 = skipWhitespace();
    result += ws1;
    if (jsonString[i] === '"') {
      result += parseString();
    }
    const ws2 = skipWhitespace();
    result += ws2;
    if (jsonString[i] === ')') {
      result += ')';
      i++;
    }
    return result;
  };

  while (i < jsonString.length) {
    const ws = skipWhitespace();
    pushSegment(ws, colors.foreground);

    if (i >= jsonString.length) break;

    const char = jsonString[i]!;

    if (char === '{' || char === '}' || char === '[' || char === ']') {
      pushSegment(char, jsonColors.bracket);
      i++;
    } else if (char === ':') {
      pushSegment(char, jsonColors.colon);
      i++;
    } else if (char === ',') {
      pushSegment(char, jsonColors.comma);
      i++;
    } else if (char === '"') {
      const str = parseString();
      const wsAfter = skipWhitespace();
      if (jsonString[i] === ':') {
        pushSegment(str, jsonColors.key);
      } else {
        pushSegment(str, jsonColors.string);
      }
      pushSegment(wsAfter, colors.foreground);
    } else if (char === '-' || /\d/.test(char)) {
      const num = parseNumber();
      pushSegment(num, jsonColors.number);
    } else if (jsonString.slice(i, i + 4) === 'true') {
      pushSegment('true', jsonColors.boolean);
      i += 4;
    } else if (jsonString.slice(i, i + 5) === 'false') {
      pushSegment('false', jsonColors.boolean);
      i += 5;
    } else if (jsonString.slice(i, i + 4) === 'null') {
      pushSegment('null', jsonColors.null);
      i += 4;
    } else if (jsonString.slice(i, i + 9) === 'ObjectId(') {
      const oid = parseObjectId();
      pushSegment(oid, jsonColors.objectId);
    } else if (jsonString.slice(i, i + 8) === 'ISODate(') {
      const date = parseISODate();
      pushSegment(date, jsonColors.date);
    } else {
      pushSegment(char, colors.foreground);
      i++;
    }
  }

  return segments;
}

export function getDocumentPreview(doc: Record<string, unknown>): string {
  const id = doc._id;
  if (id && typeof id === 'object' && '_bsontype' in id) {
    const idStr = (id as { toString(): string }).toString();
    return `{ _id: "${idStr}" }`;
  }
  return '{ _id: "..." }';
}
