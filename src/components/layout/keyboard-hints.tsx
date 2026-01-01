import { colors } from '../../theme/index.ts';
import type { FocusZone, ConnectionStatus } from '../../types/index.ts';

interface KeyboardHintsProps {
  activeZone: FocusZone;
  hasSelection: boolean;
  selectionType?: 'connection' | 'database' | 'collection' | null;
  connectionStatus?: ConnectionStatus;
}

interface Hint {
  key: string;
  action: string;
}

export function KeyboardHints({
  activeZone,
  hasSelection,
  selectionType,
  connectionStatus,
}: KeyboardHintsProps) {
  const hints = getHintsForContext(activeZone, hasSelection, selectionType, connectionStatus);

  return (
    <box height={1} paddingLeft={1} paddingRight={1}>
      <text>
        {hints.map((hint, i) => (
          <span key={hint.key}>
            <span fg={colors.actionKey}>{hint.key}</span>
            <span fg={colors.muted}> {hint.action}</span>
            {i < hints.length - 1 && <span fg={colors.dim}> </span>}
          </span>
        ))}
      </text>
    </box>
  );
}

function getHintsForContext(
  zone: FocusZone,
  hasSelection: boolean,
  selectionType?: 'connection' | 'database' | 'collection' | null,
  connectionStatus?: ConnectionStatus
): Hint[] {
  if (zone === 'modal') {
    return [
      { key: 'Tab', action: 'next' },
      { key: 'Enter', action: 'confirm' },
      { key: 'Esc', action: 'close' },
    ];
  }

  if (zone === 'sidebar') {
    const hints: Hint[] = [
      { key: '↑↓', action: 'nav' },
      { key: '←→', action: 'expand' },
      { key: 'a', action: 'add connection' },
    ];

    if (!hasSelection) {
      // No selection - just show add connection
      return hints;
    }

    if (selectionType === 'connection') {
      // Show connect or disconnect based on status
      if (connectionStatus === 'connected') {
        hints.push({ key: 'd', action: 'disconnect' });
      } else {
        hints.push({ key: 'c', action: 'connect' });
      }
      hints.push({ key: 'n', action: 'new db' });
      hints.push({ key: 'e', action: 'edit' });
      hints.push({ key: 'x', action: 'remove' });
      return hints;
    }

    if (selectionType === 'database') {
      hints.push({ key: 'n', action: 'new collection' });
      hints.push({ key: 'C', action: 'copy' });
      hints.push({ key: 'x', action: 'drop' });
      return hints;
    }

    if (selectionType === 'collection') {
      hints.push({ key: 'C', action: 'copy' });
      hints.push({ key: 'x', action: 'drop' });
      return hints;
    }

    return hints;
  }

  // Panel zone
  if (zone === 'panel') {
    if (selectionType === 'collection') {
      return [
        { key: '↑↓', action: 'nav' },
        { key: '/', action: 'filter' },
        { key: 'i', action: 'edit' },
        { key: 'C', action: 'copy' },
      ];
    }

    return [
      { key: '↑↓', action: 'scroll' },
      { key: 'Tab', action: 'switch pane' },
    ];
  }

  return [];
}
