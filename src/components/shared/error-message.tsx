import { colors } from '../../theme/index.ts';

interface ErrorMessageProps {
  message: string;
  onDismiss?: () => void;
}

export function ErrorMessage({ message, onDismiss }: ErrorMessageProps) {
  return (
    <box flexDirection="row" backgroundColor={colors.danger} paddingLeft={1} paddingRight={1}>
      <text fg={colors.selectedText}>{message}</text>
      {onDismiss && (
        <box marginLeft={2}>
          <text fg={colors.selectedText}>[x]</text>
        </box>
      )}
    </box>
  );
}
