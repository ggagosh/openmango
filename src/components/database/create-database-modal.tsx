import { useState } from 'react';
import { useKeyboard } from '@opentui/react';
import { Modal } from '../shared/modal.tsx';
import { FormField } from '../shared/form-field.tsx';
import { Button } from '../shared/button.tsx';
import { colors } from '../../theme/index.ts';

interface CreateDatabaseModalProps {
  connectionAlias: string;
  onClose: () => void;
}

type FocusField = 'name' | 'cancel' | 'create';

export function CreateDatabaseModal({ connectionAlias, onClose }: CreateDatabaseModalProps) {
  const [name, setName] = useState('');
  const [focusedField, setFocusedField] = useState<FocusField>('name');
  const [error, setError] = useState<string>();

  const fieldOrder: FocusField[] = ['name', 'cancel', 'create'];

  useKeyboard((key) => {
    if (key.name === 'tab') {
      const currentIndex = fieldOrder.indexOf(focusedField);
      const nextIndex = key.shift
        ? (currentIndex - 1 + fieldOrder.length) % fieldOrder.length
        : (currentIndex + 1) % fieldOrder.length;
      setFocusedField(fieldOrder[nextIndex]!);
    }
    if (key.name === 'escape') {
      onClose();
    }
  });

  const handleCreate = () => {
    if (!name.trim()) {
      setError('Database name is required');
      return;
    }
    // In a real app, this would create the database
    onClose();
  };

  return (
    <Modal title="Create Database" width={50} height={11}>
      <box flexDirection="column" gap={1}>
        <box marginBottom={1}>
          <text fg={colors.muted}>On: {connectionAlias}</text>
        </box>

        <FormField label="Database Name" required error={error}>
          <input
            placeholder="my_database"
            value={name}
            focused={focusedField === 'name'}
            onInput={(value) => {
              setName(value);
              setError(undefined);
            }}
            onSubmit={handleCreate}
            backgroundColor={colors.background}
            focusedBackgroundColor={colors.border}
            textColor={colors.foreground}
          />
        </FormField>

        <box flexDirection="row" justifyContent="flex-end" gap={2} marginTop={1}>
          <Button label="Cancel" focused={focusedField === 'cancel'} onPress={onClose} />
          <Button
            label="Create"
            focused={focusedField === 'create'}
            variant="primary"
            onPress={handleCreate}
          />
        </box>
      </box>
    </Modal>
  );
}
