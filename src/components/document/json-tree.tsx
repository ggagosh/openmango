import { colors } from '../../theme/index.ts';

interface JsonTreeProps {
  data: unknown;
  expandedPaths: Set<string>;
  onToggle: (path: string) => void;
}

interface JsonNodeProps {
  path: string;
  keyName: string | null;
  value: unknown;
  depth: number;
  expandedPaths: Set<string>;
  onToggle: (path: string) => void;
  isLast: boolean;
}

const jsonColors = {
  key: colors.foreground,
  string: '#4ade80',
  number: '#22d3ee',
  boolean: '#fbbf24',
  null: '#c084fc',
  bracket: colors.muted,
  objectId: colors.dim,
  date: colors.dim,
};

function getIndent(depth: number): string {
  return '  '.repeat(depth);
}

function formatValue(value: unknown): { text: string; color: string } {
  if (value === null) {
    return { text: 'null', color: jsonColors.null };
  }
  if (typeof value === 'string') {
    return { text: `"${value}"`, color: jsonColors.string };
  }
  if (typeof value === 'number') {
    return { text: String(value), color: jsonColors.number };
  }
  if (typeof value === 'boolean') {
    return { text: String(value), color: jsonColors.boolean };
  }
  if (typeof value === 'object' && '_bsontype' in (value as object)) {
    const bsonType = (value as { _bsontype: string })._bsontype;
    if (bsonType === 'ObjectId') {
      const id = (value as { toString(): string }).toString();
      return { text: `ObjectId("${id}")`, color: jsonColors.objectId };
    }
    if (bsonType === 'Date' || value instanceof Date) {
      const dateStr = (value as Date).toISOString();
      return { text: `ISODate("${dateStr}")`, color: jsonColors.date };
    }
  }
  if (value instanceof Date) {
    return { text: `ISODate("${value.toISOString()}")`, color: jsonColors.date };
  }
  // Fallback for other primitives (undefined, symbol, bigint, etc.)
  if (typeof value === 'undefined') {
    return { text: 'undefined', color: jsonColors.null };
  }
  if (typeof value === 'bigint') {
    return { text: `${value}n`, color: jsonColors.number };
  }
  return { text: JSON.stringify(value) ?? 'unknown', color: colors.foreground };
}

function isExpandable(value: unknown): boolean {
  if (value === null || typeof value !== 'object') return false;
  if ('_bsontype' in (value as object)) return false; // ObjectId, Date, etc.
  if (value instanceof Date) return false;
  return true;
}

function getCollapsedPreview(value: unknown): string {
  if (Array.isArray(value)) {
    return `[${value.length} items]`;
  }
  const keys = Object.keys(value as object);
  return `{${keys.length} fields}`;
}

function JsonNode({ path, keyName, value, depth, expandedPaths, onToggle, isLast }: JsonNodeProps) {
  const indent = getIndent(depth);
  const comma = isLast ? '' : ',';
  const expandable = isExpandable(value);
  const isExpanded = expandedPaths.has(path);

  if (!expandable) {
    const { text, color } = formatValue(value);
    return (
      <box flexDirection="row">
        <text fg={colors.foreground}>{indent}</text>
        {keyName !== null && (
          <>
            <text fg={jsonColors.key}>"{keyName}"</text>
            <text fg={jsonColors.bracket}>: </text>
          </>
        )}
        <text fg={color}>{text}</text>
        <text fg={jsonColors.bracket}>{comma}</text>
      </box>
    );
  }

  const isArray = Array.isArray(value);
  const openBracket = isArray ? '[' : '{';
  const closeBracket = isArray ? ']' : '}';
  const entries = isArray
    ? (value as unknown[]).map((v, i) => [String(i), v] as const)
    : Object.entries(value as object);

  if (!isExpanded) {
    const preview = getCollapsedPreview(value);
    return (
      <box flexDirection="row">
        <text fg={colors.foreground}>{indent}</text>
        <text fg={colors.muted}>{'>'} </text>
        {keyName !== null && (
          <>
            <text fg={jsonColors.key}>"{keyName}"</text>
            <text fg={jsonColors.bracket}>: </text>
          </>
        )}
        <text fg={jsonColors.bracket}>{openBracket}</text>
        <text fg={colors.dim}> {preview} </text>
        <text fg={jsonColors.bracket}>{closeBracket}</text>
        <text fg={jsonColors.bracket}>{comma}</text>
      </box>
    );
  }

  return (
    <box flexDirection="column">
      <box flexDirection="row">
        <text fg={colors.foreground}>{indent}</text>
        <text fg={colors.muted}>{'v'} </text>
        {keyName !== null && (
          <>
            <text fg={jsonColors.key}>"{keyName}"</text>
            <text fg={jsonColors.bracket}>: </text>
          </>
        )}
        <text fg={jsonColors.bracket}>{openBracket}</text>
      </box>
      {entries.map(([k, v], i) => (
        <JsonNode
          key={`${path}.${k}`}
          path={`${path}.${k}`}
          keyName={isArray ? null : k}
          value={v}
          depth={depth + 1}
          expandedPaths={expandedPaths}
          onToggle={onToggle}
          isLast={i === entries.length - 1}
        />
      ))}
      <box flexDirection="row">
        <text fg={colors.foreground}>{indent}</text>
        <text fg={jsonColors.bracket}>{closeBracket}</text>
        <text fg={jsonColors.bracket}>{comma}</text>
      </box>
    </box>
  );
}

export function JsonTree({ data, expandedPaths, onToggle }: JsonTreeProps) {
  if (!isExpandable(data)) {
    const { text, color } = formatValue(data);
    return <text fg={color}>{text}</text>;
  }

  return (
    <JsonNode
      path="root"
      keyName={null}
      value={data}
      depth={0}
      expandedPaths={expandedPaths}
      onToggle={onToggle}
      isLast={true}
    />
  );
}
