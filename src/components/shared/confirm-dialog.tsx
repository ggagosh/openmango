import { useState } from 'react';
import { useKeyboard } from '@opentui/react';
import { Modal } from './modal.tsx';
import { Button } from './button.tsx';
import { colors } from '../../theme/index.ts';

interface ConfirmDialogProps {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  title,
  message,
  confirmLabel = 'Confirm',
  cancelLabel = 'Cancel',
  destructive = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const [focusedButton, setFocusedButton] = useState<'cancel' | 'confirm'>('cancel');

  useKeyboard((key) => {
    if (key.name === 'tab') {
      setFocusedButton((prev) => (prev === 'cancel' ? 'confirm' : 'cancel'));
    }
    if (key.name === 'escape') {
      onCancel();
    }
    if (key.name === 'arrow-left' || key.name === 'h') {
      setFocusedButton('cancel');
    }
    if (key.name === 'arrow-right' || key.name === 'l') {
      setFocusedButton('confirm');
    }
  });

  return (
    <Modal title={title} width={50} height={10}>
      <box flexDirection="column" flexGrow={1}>
        <box flexGrow={1}>
          <text fg={colors.foreground}>{message}</text>
        </box>
        <box flexDirection="row" justifyContent="flex-end" gap={2}>
          <Button label={cancelLabel} focused={focusedButton === 'cancel'} onPress={onCancel} />
          <Button
            label={confirmLabel}
            focused={focusedButton === 'confirm'}
            variant={destructive ? 'danger' : 'primary'}
            onPress={onConfirm}
          />
        </box>
      </box>
    </Modal>
  );
}
