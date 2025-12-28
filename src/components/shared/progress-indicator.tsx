import { colors } from '../../theme/index.ts';

interface ProgressIndicatorProps {
  label?: string;
  progress?: number; // 0-100, undefined = indeterminate
  width?: number;
}

export function ProgressIndicator({ label, progress, width = 30 }: ProgressIndicatorProps) {
  const isIndeterminate = progress === undefined;
  const fillWidth = isIndeterminate ? Math.floor(width / 3) : Math.floor((progress / 100) * width);
  const emptyWidth = width - fillWidth;

  const filled = '█'.repeat(fillWidth);
  const empty = '░'.repeat(emptyWidth);

  return (
    <box flexDirection="column">
      {label && <text fg={colors.foreground}>{label}</text>}
      <box flexDirection="row">
        <text fg={colors.primary}>{filled}</text>
        <text fg={colors.border}>{empty}</text>
        {!isIndeterminate && <text fg={colors.muted}> {progress}%</text>}
      </box>
    </box>
  );
}
