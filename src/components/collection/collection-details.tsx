import type { TreeNodeData } from '../../types/index.ts';
import { useModal } from '../../context/modal-context.tsx';
import { useKeyboard } from '@opentui/react';
import { mongoService } from '../../services/mongodb.ts';
import { DocumentBrowser } from '../document/index.ts';

interface CollectionDetailsProps {
  node: TreeNodeData;
  focused: boolean;
}

export function CollectionDetails({ node, focused }: CollectionDetailsProps) {
  const { openModal } = useModal();

  useKeyboard((key) => {
    if (!focused) {
      return;
    }

    if (key.name === 'c' && key.shift) {
      openModal('copy-collection', {
        sourceConnection: node.connectionId,
        sourceDatabase: node.databaseName,
        sourceName: node.name,
      });
    }
  });

  const db =
    node.connectionId && node.databaseName
      ? mongoService.getDb(node.connectionId, node.databaseName)
      : undefined;

  if (!db) {
    return (
      <box flexGrow={1} justifyContent="center" alignItems="center">
        <text>Not connected</text>
      </box>
    );
  }

  return (
    <box flexDirection="column" flexGrow={1}>
      <DocumentBrowser db={db} collectionName={node.name} focused={focused} />
    </box>
  );
}
