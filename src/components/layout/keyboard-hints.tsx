import { colors } from '../../theme/index.ts';
import type { FocusZone } from '../../types/index.ts';

interface KeyboardHintsProps {
  activeZone: FocusZone;
  hasSelection: boolean;
  selectionType?: 'connection' | 'database' | 'collection' | null;
}

interface Hint {
  key: string;
  action: string;
}

export function KeyboardHints({ activeZone, hasSelection, selectionType }: KeyboardHintsProps) {
  const hints = getHintsForContext(activeZone, hasSelection, selectionType);

  return (
    <box height={1} paddingLeft={1} paddingRight={1}>
      <text fg={colors.muted}>
        {hints.map((hint, i) => (
          <span key={hint.key}>
            <span fg={colors.primary}>{hint.key}</span>
            <span fg={colors.dim}> {hint.action}</span>
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
  selectionType?: 'connection' | 'database' | 'collection' | null
): Hint[] {
  const common: Hint[] = [
    { action: 'switch pane', key: '1/2' },
    { action: 'search', key: '/' },
    { action: 'help', key: '?' },
  ];

  if (zone === 'modal') {
    return [
      { action: 'next field', key: 'Tab' },
      { action: 'confirm', key: 'Enter' },
      { action: 'close', key: 'Esc' },
    ];
  }

  if (zone === 'sidebar') {
    const nav: Hint[] = [
      { action: 'navigate', key: '↑↓' },
      { action: 'expand', key: '←→' },
    ];

    if (!hasSelection) {
      return [...nav, { action: 'new connection', key: 'n' }, ...common];
    }

    if (selectionType === 'connection') {
      return [
        ...nav,
        { action: 'connect', key: 'c' },
        { action: 'disconnect', key: 'd' },
        { action: 'new db', key: 'n' },
        ...common,
      ];
    }

    if (selectionType === 'database') {
      return [
        ...nav,
        { action: 'new collection', key: 'n' },
        { action: 'copy', key: 'C' },
        { action: 'drop', key: 'Del' },
        ...common,
      ];
    }

    if (selectionType === 'collection') {
      return [...nav, { action: 'copy', key: 'C' }, { action: 'drop', key: 'Del' }, ...common];
    }

    return [...nav, ...common];
  }

  return [{ action: 'scroll', key: '↑↓' }, ...common];
}
