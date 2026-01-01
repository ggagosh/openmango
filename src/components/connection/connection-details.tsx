import type { TreeNodeData } from '../../types/index.ts';
import { useApp } from '../../context/app-context.tsx';
import { useModal } from '../../context/modal-context.tsx';
import { useConnection } from '../../hooks/use-connection.ts';
import { removeConnection } from '../../services/index.ts';
import { useKeyboard } from '@opentui/react';
import { colors, icons } from '../../theme/index.ts';
import { ActionRow, Section, StatRow } from '../shared/section.tsx';

interface ConnectionDetailsProps {
  node: TreeNodeData;
  focused: boolean;
}

export function ConnectionDetails({ node, focused }: ConnectionDetailsProps) {
  const { state, dispatch } = useApp();
  const { openModal } = useModal();
  const { connect, disconnect } = useConnection();

  const connection = state.connections.find((c) => c.id === node.connectionId);
  const connectionId = node.connectionId;

  const handleRemove = async () => {
    if (!connectionId) return;
    // Disconnect if connected
    if (connection?.status === 'connected') {
      await disconnect(connectionId);
    }
    // Remove from persistence and state
    await removeConnection(connectionId);
    dispatch({ type: 'REMOVE_CONNECTION', payload: connectionId });
  };

  useKeyboard((key) => {
    if (!focused || !connectionId) return;

    if (key.name === 'n') {
      openModal('create-database', { connectionAlias: node.name });
    } else if (key.name === 'c' && connection?.status === 'disconnected') {
      void connect(connectionId);
    } else if (key.name === 'd' && connection?.status === 'connected') {
      void disconnect(connectionId);
    } else if (key.name === 'e') {
      openModal('edit-connection', { connectionId });
    } else if (key.name === 'x') {
      openModal('confirm', {
        title: 'Remove Connection',
        message: `Remove connection "${connection?.alias ?? node.name}"?`,
        confirmLabel: 'Remove',
        destructive: true,
        onConfirm: handleRemove,
      });
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
            { key: 'c', label: 'Connect', disabled: connection?.status === 'connected' },
            { key: 'd', label: 'Disconnect', disabled: connection?.status !== 'connected' },
            { key: 'e', label: 'Edit' },
            { key: 'x', label: 'Remove' },
          ]}
        />
      </Section>
    </box>
  );
}
