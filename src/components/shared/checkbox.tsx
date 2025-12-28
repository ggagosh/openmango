import { useKeyboard } from '@opentui/react';
import { colors } from '../../theme/index.ts';

interface CheckboxProps {
  label: string;
  checked: boolean;
  focused?: boolean;
  onChange: (checked: boolean) => void;
}

export function Checkbox({ label, checked, focused = false, onChange }: CheckboxProps) {
  useKeyboard((key) => {
    if (!focused) {
      return;
    }
    if (key.name === 'space' || key.name === 'return') {
      onChange(!checked);
    }
  });

  const boxColor = focused ? colors.primary : colors.foreground;
  const checkDisplay = checked ? '[x]' : '[ ]';

  return (
    <box flexDirection="row">
      <text fg={boxColor}>{checkDisplay}</text>
      <text fg={colors.foreground}> {label}</text>
    </box>
  );
}
