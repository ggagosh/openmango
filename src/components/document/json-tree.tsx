import { colors } from '../../theme/index.ts';

interface JsonTreeProps {
  data: unknown;
  expandedPaths: Set<string>;
  selectedPath: string | null;
  editingPath: string | null;
  editingValue: string;
  onToggle: (path: string) => void;
  onEditChange: (value: string) => void;
}

interface JsonNodeProps {
  path: string;
  keyName: string | null;
  value: unknown;
  depth: number;
  expandedPaths: Set<string>;
  selectedPath: string | null;
  editingPath: string | null;
  editingValue: string;
  onToggle: (path: string) => void;
  onEditChange: (value: string) => void;
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
  if (typeof value === 'undefined') {
    return { text: 'undefined', color: jsonColors.null };
  }
  if (typeof value === 'bigint') {
    return { text: `${value}n`, color: jsonColors.number };
  }
  return { text: JSON.stringify(value) ?? 'unknown', color: colors.foreground };
}

export function isExpandable(value: unknown): boolean {
  if (value === null || typeof value !== 'object') return false;
  if ('_bsontype' in (value as object)) return false;
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

/**
 * Flatten the tree into a list of visible paths for navigation.
 * Only includes paths that are currently visible (respects expandedPaths).
 */
export function flattenVisiblePaths(
  data: unknown,
  expandedPaths: Set<string>,
  path = 'root'
): string[] {
  const paths: string[] = [path];

  if (!isExpandable(data) || !expandedPaths.has(path)) {
    return paths;
  }

  const entries = Array.isArray(data)
    ? (data as unknown[]).map((v, i) => [String(i), v] as const)
    : Object.entries(data as object);

  for (const [key, value] of entries) {
    const childPath = `${path}.${key}`;
    paths.push(...flattenVisiblePaths(value, expandedPaths, childPath));
  }

  return paths;
}

/**
 * Get a value at a specific path in the data structure.
 * Path format: "root.field.nested" or "root.0.field" for arrays.
 */
export function getValueAtPath(data: unknown, path: string): unknown {
  const parts = path.split('.');
  let current: unknown = data;

  // Skip 'root' prefix
  for (let i = 1; i < parts.length; i++) {
    const key = parts[i]!;
    if (current === null || typeof current !== 'object') {
      return undefined;
    }
    current = (current as Record<string, unknown>)[key];
  }

  return current;
}

/**
 * Set a value at a specific path, returning a new object (immutable).
 */
export function setValueAtPath(data: unknown, path: string, newValue: unknown): unknown {
  const parts = path.split('.').slice(1); // Remove 'root'
  if (parts.length === 0) return newValue;

  const result = JSON.parse(JSON.stringify(data)) as Record<string, unknown>;
  let current: Record<string, unknown> = result;

  for (let i = 0; i < parts.length - 1; i++) {
    const key = parts[i]!;
    current = current[key] as Record<string, unknown>;
  }

  current[parts[parts.length - 1]!] = newValue;
  return result;
}

function JsonNode({
  path,
  keyName,
  value,
  depth,
  expandedPaths,
  selectedPath,
  editingPath,
  editingValue,
  onToggle,
  onEditChange,
  isLast,
}: JsonNodeProps) {
  const indent = getIndent(depth);
  const comma = isLast ? '' : ',';
  const expandable = isExpandable(value);
  const isExpanded = expandedPaths.has(path);
  const isSelected = path === selectedPath;
  const isEditing = path === editingPath;

  // Selection highlight colors
  const bgColor = isSelected ? colors.selected : undefined;
  const selectedIndicator = isSelected ? '>' : ' ';

  // Editing mode for primitive values
  if (isEditing && !expandable) {
    return (
      <box flexDirection="row" backgroundColor={bgColor}>
        <text fg={colors.primary}>{selectedIndicator}</text>
        <text fg={colors.foreground}>{indent}</text>
        {keyName !== null && (
          <>
            <text fg={jsonColors.key}>"{keyName}"</text>
            <text fg={jsonColors.bracket}>: </text>
          </>
        )}
        <text fg={colors.warning}>{editingValue}</text>
        <text fg={colors.dim}>_</text>
        <text fg={jsonColors.bracket}>{comma}</text>
      </box>
    );
  }

  // Non-expandable (primitive) values
  if (!expandable) {
    const { text, color } = formatValue(value);
    return (
      <box flexDirection="row" backgroundColor={bgColor}>
        <text fg={colors.primary}>{selectedIndicator}</text>
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

  // Collapsed expandable
  if (!isExpanded) {
    const preview = getCollapsedPreview(value);
    return (
      <box flexDirection="row" backgroundColor={bgColor}>
        <text fg={colors.primary}>{selectedIndicator}</text>
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

  // Expanded expandable
  return (
    <box flexDirection="column">
      <box flexDirection="row" backgroundColor={bgColor}>
        <text fg={colors.primary}>{selectedIndicator}</text>
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
          selectedPath={selectedPath}
          editingPath={editingPath}
          editingValue={editingValue}
          onToggle={onToggle}
          onEditChange={onEditChange}
          isLast={i === entries.length - 1}
        />
      ))}
      <box flexDirection="row">
        <text fg={colors.foreground}> {indent}</text>
        <text fg={jsonColors.bracket}>{closeBracket}</text>
        <text fg={jsonColors.bracket}>{comma}</text>
      </box>
    </box>
  );
}

export function JsonTree({
  data,
  expandedPaths,
  selectedPath,
  editingPath,
  editingValue,
  onToggle,
  onEditChange,
}: JsonTreeProps) {
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
      selectedPath={selectedPath}
      editingPath={editingPath}
      editingValue={editingValue}
      onToggle={onToggle}
      onEditChange={onEditChange}
      isLast={true}
    />
  );
}
