import type { Document, WithId } from 'mongodb';
import { useDocument } from '../../context/document-context.tsx';
import { colors } from '../../theme/index.ts';
import { JsonTree } from './json-tree.tsx';

interface DocumentExpandedProps {
  document: WithId<Document>;
  isEditing: boolean;
  editBuffer?: string;
  hasChanges?: boolean;
}

export function DocumentExpanded({ document }: DocumentExpandedProps) {
  const { state, dispatch } = useDocument();
  const {
    treeExpandedPaths,
    selectedPath,
    editingPath,
    editingValue,
    editingValueType,
    pendingDocument,
    treeNavigationActive,
    editingId,
    hasUnsavedChanges,
  } = state;

  // Check if this document is the one being navigated
  const isThisDocActive = editingId === document._id.toString();

  // Use pending document if this is the active document and we have pending changes
  const displayDoc = isThisDocActive && pendingDocument ? pendingDocument : document;

  const handleToggle = (path: string) => {
    dispatch({ type: 'TOGGLE_TREE_PATH', payload: path });
  };

  const handleEditChange = (value: string) => {
    dispatch({ type: 'UPDATE_FIELD_VALUE', payload: value });
  };

  // Determine border color based on state
  let borderColor = colors.sectionBorder;
  if (isThisDocActive) {
    if (hasUnsavedChanges) {
      borderColor = colors.warning;
    } else if (treeNavigationActive) {
      borderColor = colors.primary;
    }
  }

  return (
    <box
      flexDirection="column"
      border
      borderStyle="rounded"
      borderColor={borderColor}
      marginLeft={2}
      marginRight={1}
      paddingLeft={1}
    >
      <JsonTree
        data={displayDoc}
        expandedPaths={isThisDocActive ? treeExpandedPaths : new Set(['root'])}
        selectedPath={isThisDocActive && treeNavigationActive ? selectedPath : null}
        editingPath={isThisDocActive ? editingPath : null}
        editingValue={isThisDocActive ? editingValue : ''}
        editingValueType={isThisDocActive ? editingValueType : null}
        onToggle={handleToggle}
        onEditChange={handleEditChange}
      />
      {isThisDocActive && hasUnsavedChanges && (
        <box paddingTop={1}>
          <text fg={colors.warning}>* unsaved changes</text>
        </box>
      )}
    </box>
  );
}
