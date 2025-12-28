import { colors } from '../../theme/index.ts';

interface StatusBarProps {
  connectionCount: number;
  selectedItem?: string;
  message?: string;
  messageType?: 'info' | 'error' | 'success';
}

export function StatusBar({
  connectionCount,
  selectedItem,
  message,
  messageType = 'info',
}: StatusBarProps) {
  const messageColor =
    messageType === 'error'
      ? colors.statusError
      : messageType === 'success'
        ? colors.success
        : colors.muted;

  return (
    <box height={1} flexDirection="row" paddingLeft={1} paddingRight={1}>
      <text fg={colors.muted}>
        Connected: <span fg={colors.statusConnected}>{connectionCount}</span>
      </text>
      {selectedItem && <text fg={colors.muted}> │ Selected: {selectedItem}</text>}
      {message && (
        <box flexGrow={1} justifyContent="flex-end">
          <text fg={messageColor}>{message}</text>
        </box>
      )}
    </box>
  );
}
