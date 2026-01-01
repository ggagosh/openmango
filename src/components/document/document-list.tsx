import { useDocument } from '../../context/document-context.tsx';
import { colors } from '../../theme/index.ts';
import { DocumentItem } from './document-item.tsx';
import { DocumentExpanded } from './document-expanded.tsx';

interface DocumentListProps {
  maxHeight?: number;
}

export function DocumentList({ maxHeight }: DocumentListProps) {
  const { state } = useDocument();
  const { documents, selectedIndex, expandedIds, editingId, editBuffer, hasUnsavedChanges } = state;

  if (documents.length === 0) {
    return (
      <box paddingLeft={1} paddingTop={1}>
        <text fg={colors.muted}>No documents found</text>
      </box>
    );
  }

  return (
    <box flexDirection="column" height={maxHeight}>
      {documents.map((doc, index) => {
        const docId = doc._id.toString();
        const isSelected = index === selectedIndex;
        const isExpanded = expandedIds.has(docId);
        const isEditing = editingId === docId;

        return (
          <box key={docId} flexDirection="column">
            <DocumentItem document={doc} isSelected={isSelected} isExpanded={isExpanded} />
            {isExpanded && (
              <DocumentExpanded
                document={doc}
                isEditing={isEditing}
                editBuffer={isEditing ? editBuffer : undefined}
                hasChanges={isEditing && hasUnsavedChanges}
              />
            )}
          </box>
        );
      })}
    </box>
  );
}
