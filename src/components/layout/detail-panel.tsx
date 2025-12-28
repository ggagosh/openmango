import { useApp } from '../../context/app-context.tsx';
import { DatabaseDetails } from '../database/database-details.tsx';
import { CollectionDetails } from '../collection/collection-details.tsx';
import { ConnectionDetails } from '../connection/connection-details.tsx';
import { colors } from '../../theme/index.ts';

interface DetailPanelProps {
  focused: boolean;
}

export function DetailPanel({ focused }: DetailPanelProps) {
  const { state } = useApp();

  if (!state.selectedNodeId) {
    return (
      <box flexGrow={1} justifyContent="center" alignItems="center">
        <text fg={colors.muted}>Select an item from the sidebar</text>
      </box>
    );
  }

  const selectedNode = findNode(state.navigationTree, state.selectedNodeId);
  if (!selectedNode) {
    return (
      <box flexGrow={1} justifyContent="center" alignItems="center">
        <text fg={colors.muted}>Item not found</text>
      </box>
    );
  }

  switch (selectedNode.type) {
    case 'connection':
      return <ConnectionDetails node={selectedNode} focused={focused} />;
    case 'database':
      return <DatabaseDetails node={selectedNode} focused={focused} />;
    case 'collection':
      return <CollectionDetails node={selectedNode} focused={focused} />;
    default:
      return null;
  }
}

function findNode(
  nodes: import('../../types/index.ts').TreeNodeData[],
  id: string | null
): import('../../types/index.ts').TreeNodeData | null {
  if (!id) {
    return null;
  }
  for (const node of nodes) {
    if (node.id === id) {
      return node;
    }
    const found = findNode(node.children, id);
    if (found) {
      return found;
    }
  }
  return null;
}
