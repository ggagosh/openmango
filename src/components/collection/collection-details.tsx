import type { TreeNodeData } from '../../types/index.ts';
import { useModal } from '../../context/modal-context.tsx';
import { ActionRow, Section, StatRow } from '../shared/section.tsx';
import { useKeyboard } from '@opentui/react';

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

  const stats = {
    avgDocumentSize: '2.3 KB',
    documents: 45123,
    indexes: 4,
    totalSize: '104 MB',
  };

  return (
    <box flexDirection="column" gap={1}>
      <Section title="Stats">
        <StatRow label="Database" value={node.databaseName ?? ''} />
        <StatRow label="Connection" value={node.connectionId ?? ''} />
        <StatRow label="Documents" value={stats.documents.toLocaleString()} />
        <StatRow label="Avg. Size" value={stats.avgDocumentSize} />
        <StatRow label="Total Size" value={stats.totalSize} />
        <StatRow label="Indexes" value={stats.indexes} />
      </Section>

      <Section title="Actions">
        <ActionRow
          actions={[
            { key: 'C', label: 'Copy' },
            { key: 'Del', label: 'Drop' },
          ]}
        />
      </Section>
    </box>
  );
}
