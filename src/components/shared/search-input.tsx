import { useEffect, useState } from 'react';
import { useKeyboard } from '@opentui/react';
import { colors } from '../../theme/index.ts';

interface SearchInputProps {
  value: string;
  onChange: (value: string) => void;
  onClose: () => void;
  placeholder?: string;
}

export function SearchInput({
  value,
  onChange,
  onClose,
  placeholder = 'Search...',
}: SearchInputProps) {
  const [localValue, setLocalValue] = useState(value);

  useEffect(() => {
    setLocalValue(value);
  }, [value]);

  useKeyboard((key) => {
    if (key.name === 'escape') {
      onClose();
      return;
    }

    if (key.name === 'return') {
      onChange(localValue);
      onClose();
      return;
    }

    if (key.name === 'backspace') {
      setLocalValue((v) => v.slice(0, -1));
      onChange(localValue.slice(0, -1));
      return;
    }

    if (key.name && key.name.length === 1 && !key.ctrl && !key.meta) {
      const newValue = localValue + key.name;
      setLocalValue(newValue);
      onChange(newValue);
    }
  });

  return (
    <box height={1} paddingLeft={1}>
      <text fg={colors.primary}>/</text>
      <text fg={colors.foreground}>
        {localValue || <span fg={colors.muted}>{placeholder}</span>}
      </text>
      <text fg={colors.primary}>▌</text>
    </box>
  );
}
