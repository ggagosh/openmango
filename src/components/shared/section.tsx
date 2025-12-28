import type { ReactNode } from 'react';
import { colors } from '../../theme/index.ts';

interface SectionProps {
  title: string;
  children: ReactNode;
  width?: number | `${number}%` | 'auto';
}

export function Section({ title, children, width }: SectionProps) {
  return (
    <box
      flexDirection="column"
      width={width}
      border
      borderStyle="rounded"
      borderColor={colors.sectionBorder}
    >
      <box paddingLeft={1}>
        <text fg={colors.sectionTitle}>{title}</text>
      </box>
      <box flexDirection="column" paddingLeft={1} paddingRight={1}>
        {children}
      </box>
    </box>
  );
}

interface StatRowProps {
  label: string;
  value: string | number;
}

export function StatRow({ label, value }: StatRowProps) {
  return (
    <box flexDirection="row">
      <text fg={colors.muted}>{label}</text>
      <box flexGrow={1} />
      <text fg={colors.foreground}>{value}</text>
    </box>
  );
}

interface ActionRowProps {
  actions: { key: string; label: string }[];
}

export function ActionRow({ actions }: ActionRowProps) {
  return (
    <box flexDirection="row" gap={2}>
      {actions.map((action) => (
        <text key={action.key} fg={colors.foreground}>
          <span fg={colors.primary}>[{action.key}]</span> {action.label}
        </text>
      ))}
    </box>
  );
}
