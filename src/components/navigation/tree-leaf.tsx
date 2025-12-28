import type { TreeNodeData } from '../../types/index.ts';
import { colors, icons, layout } from '../../theme/index.ts';
import { truncate } from '../../utils/truncate.ts';

interface TreeLeafProps {
  node: TreeNodeData;
  depth: number;
  isSelected: boolean;
  isLast?: boolean;
  prefix?: string;
}

export function TreeLeaf({ node, isSelected, isLast = false, prefix = '' }: TreeLeafProps) {
  const linePrefix = prefix + (isLast ? icons.treeEnd : icons.treeBranch);

  const bgColor = isSelected ? colors.selected : undefined;
  const fgColor = isSelected ? colors.selectedText : colors.foreground;

  // Calculate available width for name (sidebar - prefix - padding)
  const prefixWidth = linePrefix.length;
  const padding = 2;
  const maxNameWidth = layout.sidebarWidth - prefixWidth - padding;
  const displayName = truncate(node.name, Math.max(maxNameWidth, 8));

  return (
    <box backgroundColor={bgColor}>
      <text fg={fgColor}>
        <span fg={colors.dim}>{linePrefix}</span>
        <span>{displayName}</span>
      </text>
    </box>
  );
}
