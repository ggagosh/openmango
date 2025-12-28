import { useKeyboard } from '@opentui/react';
import { colors } from '../../theme/index.ts';

interface ButtonProps {
  label: string;
  focused?: boolean;
  variant?: 'primary' | 'secondary' | 'danger';
  onPress: () => void;
  disabled?: boolean;
}

export function Button({
  label,
  focused = false,
  variant = 'secondary',
  onPress,
  disabled = false,
}: ButtonProps) {
  useKeyboard((key) => {
    if (!focused || disabled) {
      return;
    }
    if (key.name === 'return' || key.name === 'space') {
      onPress();
    }
  });

  let bgColor: string;
  let fgColor: string;

  if (disabled) {
    bgColor = colors.border;
    fgColor = colors.dim;
  } else if (focused) {
    switch (variant) {
      case 'primary':
        bgColor = colors.primary;
        fgColor = colors.selectedText;
        break;
      case 'danger':
        bgColor = colors.danger;
        fgColor = colors.selectedText;
        break;
      default:
        bgColor = colors.borderFocused;
        fgColor = colors.foreground;
    }
  } else {
    bgColor = colors.border;
    fgColor = colors.foreground;
  }

  return (
    <box backgroundColor={bgColor} paddingLeft={2} paddingRight={2}>
      <text fg={fgColor}>{label}</text>
    </box>
  );
}
