import type { ReactNode } from 'react';
import type { ConnectionStatus, TreeNodeData } from '../../types/index.ts';
import { colors, icons, layout } from '../../theme/index.ts';
import { truncate } from '../../utils/truncate.ts';

interface TreeNodeProps {
  node: TreeNodeData;
  depth: number;
  isSelected: boolean;
  isLast?: boolean;
  prefix?: string;
  children?: ReactNode;
}

function getStatusIcon(status?: ConnectionStatus): { icon: string; color: string } {
  switch (status) {
    case 'connected':
      return { color: colors.statusConnected, icon: icons.connected };
    case 'connecting':
      return { color: colors.statusConnecting, icon: icons.connecting };
    case 'error':
      return { color: colors.statusError, icon: icons.error };
    case 'disconnected':
    default:
      return { color: colors.statusDisconnected, icon: icons.disconnected };
  }
}

export function TreeNode({
  node,
  depth,
  isSelected,
  isLast = false,
  prefix = '',
  children,
}: TreeNodeProps) {
  const expandIcon = node.isExpanded ? icons.expanded : icons.collapsed;
  const linePrefix = depth > 0 ? prefix + (isLast ? icons.treeEnd : icons.treeBranch) : '';
  const isConnection = node.type === 'connection';
  const statusInfo = isConnection ? getStatusIcon(node.status) : null;
  const bgColor = isSelected ? colors.selected : undefined;
  const fgColor = isSelected ? colors.selectedText : colors.foreground;

  // Calculate available width for name (sidebar - prefix - expand icon - status icon - padding)
  const prefixWidth = linePrefix.length;
  const expandIconWidth = 2; // icon + space
  const statusIconWidth = statusInfo ? 5 : 0; // space + icon
  const padding = 2;
  const maxNameWidth =
    layout.sidebarWidth - prefixWidth - expandIconWidth - statusIconWidth - padding;
  const displayName = truncate(node.name, Math.max(maxNameWidth, 8));

  return (
    <box flexDirection="column">
      <box backgroundColor={bgColor} flexDirection="row">
        <text fg={fgColor}>
          <span fg={colors.dim}>{linePrefix}</span>
          <span>
            {expandIcon} {displayName}
          </span>
        </text>
        {statusInfo && (
          <box flexGrow={1} justifyContent="flex-end">
            <text fg={statusInfo.color}> {statusInfo.icon}</text>
          </box>
        )}
      </box>
      {node.isExpanded && children}
    </box>
  );
}

export function getChildPrefix(depth: number, isLast: boolean, parentPrefix: string): string {
  if (depth === 0) {
    return '';
  }
  return parentPrefix + (isLast ? icons.treeIndent : icons.treeVertical);
}
