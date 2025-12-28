import type { ReactNode } from 'react';
import { colors } from '../../theme/index.ts';

interface PanelProps {
  title?: string;
  sectionNumber?: number;
  focused: boolean;
  children?: ReactNode;
}

export function Panel({ title = 'Details', sectionNumber = 2, focused, children }: PanelProps) {
  const borderColor = focused ? colors.borderFocused : colors.border;
  const titleColor = focused ? colors.success : colors.muted;

  return (
    <box flexGrow={1} flexDirection="column" border borderStyle="single" borderColor={borderColor}>
      <box>
        <text fg={titleColor}>
          [{sectionNumber}] {title}
        </text>
      </box>
      <box flexDirection="column" flexGrow={1} padding={1}>
        {children}
      </box>
    </box>
  );
}
