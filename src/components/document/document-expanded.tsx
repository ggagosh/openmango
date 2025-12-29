import { useState, useCallback } from 'react';
import type { Document, WithId } from 'mongodb';
import { colors } from '../../theme/index.ts';
import { highlightJson } from '../../utils/json-highlighter.ts';
import { formatDocument } from '../../utils/json-formatter.ts';
import { JsonTree } from './json-tree.tsx';

interface DocumentExpandedProps {
  document: WithId<Document>;
  isEditing: boolean;
  editBuffer?: string;
  hasChanges?: boolean;
}

export function DocumentExpanded({
  document,
  isEditing,
  editBuffer,
  hasChanges,
}: DocumentExpandedProps) {
  // Expand root by default
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(() => new Set(['root']));

  const handleToggle = useCallback((path: string) => {
    setExpandedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }, []);

  // Edit mode uses flat text representation
  if (isEditing) {
    const jsonString = editBuffer !== undefined ? editBuffer : formatDocument(document);
    const lines = jsonString.split('\n');

    return (
      <box
        flexDirection="column"
        border
        borderStyle="rounded"
        borderColor={colors.warning}
        marginLeft={2}
        marginRight={1}
      >
        {lines.map((line, lineIndex) => {
          const segments = highlightJson(line);
          const lineKey = `line-${lineIndex}-${line.slice(0, 20)}`;
          return (
            <box key={lineKey} flexDirection="row" paddingLeft={1}>
              {segments.map((segment, segIndex) => (
                <text key={`${lineKey}-seg-${segIndex}`} fg={segment.color}>
                  {segment.text}
                </text>
              ))}
            </box>
          );
        })}
        {hasChanges && (
          <box paddingLeft={1} paddingTop={1}>
            <text fg={colors.warning}>* unsaved changes</text>
          </box>
        )}
      </box>
    );
  }

  // View mode uses collapsible JSON tree
  return (
    <box
      flexDirection="column"
      border
      borderStyle="rounded"
      borderColor={colors.sectionBorder}
      marginLeft={2}
      marginRight={1}
      paddingLeft={1}
    >
      <JsonTree data={document} expandedPaths={expandedPaths} onToggle={handleToggle} />
    </box>
  );
}
