import type { TreeNodeData } from '../../types/index.ts';
import { useModal } from '../../context/modal-context.tsx';
import { useKeyboard } from '@opentui/react';
import { ActionRow, Section, StatRow } from '../shared/section.tsx';

interface DatabaseDetailsProps {
  node: TreeNodeData;
  focused: boolean;
}

export function DatabaseDetails({ node, focused }: DatabaseDetailsProps) {
  const { openModal } = useModal();

  useKeyboard((key) => {
    if (!focused) {
      return;
    }

    if (key.name === 'n') {
      openModal('create-collection', {
        connectionAlias: node.connectionId,
        databaseName: node.name,
      });
    }
    if (key.name === 'c' && key.shift) {
      openModal('copy-database', {
        sourceConnection: node.connectionId,
        sourceName: node.name,
      });
    }
  });

  const stats = {
    collections: node.children.length,
    documents: 145892,
    indexes: 12,
    sizeOnDisk: '256 MB',
  };

  return (
    <box flexDirection="column" gap={1}>
      <Section title="Stats">
        <StatRow label="Connection" value={node.connectionId ?? ''} />
        <StatRow label="Collections" value={stats.collections} />
        <StatRow label="Documents" value={stats.documents.toLocaleString()} />
        <StatRow label="Size on Disk" value={stats.sizeOnDisk} />
        <StatRow label="Indexes" value={stats.indexes} />
      </Section>

      <Section title="Actions">
        <ActionRow
          actions={[
            { key: 'n', label: 'New Collection' },
            { key: 'C', label: 'Copy' },
            { key: 'Del', label: 'Drop' },
          ]}
        />
      </Section>
    </box>
  );
}
