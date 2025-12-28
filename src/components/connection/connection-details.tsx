import type { TreeNodeData } from '../../types/index.ts';
import { useApp } from '../../context/app-context.tsx';
import { useModal } from '../../context/modal-context.tsx';
import { useKeyboard } from '@opentui/react';
import { colors, icons } from '../../theme/index.ts';
import { ActionRow, Section, StatRow } from '../shared/section.tsx';

interface ConnectionDetailsProps {
  node: TreeNodeData;
  focused: boolean;
}

export function ConnectionDetails({ node, focused }: ConnectionDetailsProps) {
  const { state } = useApp();
  const { openModal } = useModal();

  const connection = state.connections.find((c) => c.id === node.connectionId);

  useKeyboard((key) => {
    if (!focused) {
      return;
    }

    if (key.name === 'n') {
      openModal('create-database', { connectionAlias: node.name });
    }
  });

  const statusColor =
    connection?.status === 'connected'
      ? colors.statusConnected
      : connection?.status === 'error'
        ? colors.statusError
        : colors.statusDisconnected;

  const statusIcon =
    connection?.status === 'connected'
      ? icons.connected
      : connection?.status === 'error'
        ? icons.error
        : icons.disconnected;

  const statusText = `${statusIcon} ${connection?.status ?? 'unknown'}`;

  return (
    <box flexDirection="column" gap={1}>
      <Section title="Info">
        <StatRow label="URL" value={connection?.url ?? 'Unknown'} />
        <box flexDirection="row">
          <text fg={colors.muted}>Status</text>
          <box flexGrow={1} />
          <text fg={statusColor}>{statusText}</text>
        </box>
        <StatRow label="Databases" value={node.children.length} />
      </Section>

      {connection?.error && (
        <Section title="Error">
          <text fg={colors.statusError}>{connection.error}</text>
        </Section>
      )}

      <Section title="Actions">
        <ActionRow
          actions={[
            { key: 'n', label: 'New Database' },
            { key: 'c', label: 'Connect' },
            { key: 'd', label: 'Disconnect' },
          ]}
        />
      </Section>
    </box>
  );
}
