import type { Document, WithId } from 'mongodb';
import { colors, icons } from '../../theme/index.ts';
import { getDocumentPreview } from '../../utils/json-highlighter.ts';

interface DocumentItemProps {
  document: WithId<Document>;
  isSelected: boolean;
  isExpanded: boolean;
}

export function DocumentItem({ document, isSelected, isExpanded }: DocumentItemProps) {
  const preview = getDocumentPreview(document as Record<string, unknown>);
  const icon = isExpanded ? icons.expanded : icons.collapsed;

  const bgColor = isSelected ? colors.selected : undefined;
  const textColor = isSelected ? colors.selectedText : colors.foreground;

  return (
    <box flexDirection="row" backgroundColor={bgColor} paddingLeft={1} paddingRight={1}>
      <text fg={colors.muted}>{icon} </text>
      <text fg={textColor}>{preview}</text>
    </box>
  );
}
