import { useState } from 'react';
import { useKeyboard } from '@opentui/react';
import { Modal } from '../shared/modal.tsx';
import { FormField } from '../shared/form-field.tsx';
import { Button } from '../shared/button.tsx';
import { colors } from '../../theme/index.ts';

interface CreateCollectionModalProps {
  databaseName: string;
  connectionAlias: string;
  onClose: () => void;
}

type FocusField = 'name' | 'cancel' | 'create';

export function CreateCollectionModal({
  databaseName,
  connectionAlias,
  onClose,
}: CreateCollectionModalProps) {
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
      setError('Collection name is required');
      return;
    }
    // In a real app, this would create the collection
    onClose();
  };

  return (
    <Modal title="Create Collection" width={50} height={12}>
      <box flexDirection="column" gap={1}>
        <box marginBottom={1} flexDirection="column">
          <text fg={colors.muted}>Database: {databaseName}</text>
          <text fg={colors.muted}>Connection: {connectionAlias}</text>
        </box>

        <FormField label="Collection Name" required error={error}>
          <input
            placeholder="my_collection"
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
