import type { TreeNodeData } from '../../types/index.ts';
import { mongoService } from '../../services/mongodb.ts';
import { DocumentBrowser } from '../document/index.ts';

interface CollectionDetailsProps {
  node: TreeNodeData;
  focused: boolean;
}

export function CollectionDetails({ node, focused }: CollectionDetailsProps) {
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
      <DocumentBrowser
        db={db}
        collectionName={node.name}
        connectionId={node.connectionId}
        focused={focused}
      />
    </box>
  );
}
