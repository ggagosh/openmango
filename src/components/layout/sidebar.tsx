import type { ReactNode } from 'react';
import { colors, layout } from '../../theme/index.ts';
import { truncate } from '../../utils/truncate.ts';

interface SidebarProps {
  title?: string;
  sectionNumber?: number;
  focused: boolean;
  itemCount?: number;
  selectedIndex?: number;
  breadcrumb?: string | null;
  children?: ReactNode;
}

export function Sidebar({
  title = 'Connections',
  sectionNumber = 1,
  focused,
  itemCount = 0,
  selectedIndex = 0,
  breadcrumb,
  children,
}: SidebarProps) {
  const borderColor = focused ? colors.borderFocused : colors.border;
  const titleColor = focused ? colors.success : colors.muted;
  const countText = itemCount > 0 ? `${selectedIndex + 1} of ${itemCount}` : '';

  // Show breadcrumb when available, otherwise show default title
  // Reserve space for borders (2) + padding (2) = max width is sidebarWidth - 4
  const maxTitleWidth = layout.sidebarWidth - 4;
  const displayTitle = breadcrumb
    ? truncate(breadcrumb, maxTitleWidth)
    : `[${sectionNumber}] ${title}`;

  return (
    <box
      width={layout.sidebarWidth}
      flexDirection="column"
      border
      borderStyle="single"
      borderColor={borderColor}
    >
      <box height={1} flexDirection="row" justifyContent="space-between">
        <text fg={titleColor}>{displayTitle}</text>
      </box>
      <box flexDirection="column" flexGrow={1}>
        {children}
      </box>
      {countText && (
        <box height={1} justifyContent="flex-end">
          <text fg={titleColor}>{countText}</text>
        </box>
      )}
    </box>
  );
}
