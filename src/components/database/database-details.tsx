import { useCallback, useEffect, useState } from 'react';
import { useKeyboard } from '@opentui/react';
import type { TreeNodeData, DatabaseStats } from '../../types/index.ts';
import { useModal } from '../../context/modal-context.tsx';
import { mongoService } from '../../services/mongodb.ts';
import { getDatabaseStats } from '../../services/database.ts';
import { ActionRow, Section, StatRow } from '../shared/section.tsx';

interface DatabaseDetailsProps {
  node: TreeNodeData;
  focused: boolean;
}

export function DatabaseDetails({ node, focused }: DatabaseDetailsProps) {
  const { openModal } = useModal();
  const [stats, setStats] = useState<DatabaseStats | null>(null);

  const loadStats = useCallback(async () => {
    if (!node.connectionId || !node.name) return;
    const client = mongoService.getClient(node.connectionId);
    if (!client) return;

    try {
      const dbStats = await getDatabaseStats(client, node.name);
      setStats(dbStats);
    } catch {
      setStats(null);
    }
  }, [node.connectionId, node.name]);

  useEffect(() => {
    setStats(null);
    void loadStats();
  }, [loadStats]);

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

  return (
    <box flexDirection="column" gap={1}>
      <Section title="Stats">
        <StatRow label="Connection" value={node.connectionId ?? ''} />
        <StatRow label="Collections" value={node.children.length} />
        <StatRow
          label="Documents"
          value={stats ? stats.documentCount.toLocaleString() : 'Loading...'}
        />
        <StatRow label="Size on Disk" value={stats?.sizeOnDisk ?? 'Loading...'} />
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
