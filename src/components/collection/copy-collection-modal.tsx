import { useState } from 'react';
import { useKeyboard } from '@opentui/react';
import { useApp } from '../../context/app-context.tsx';
import { Modal } from '../shared/modal.tsx';
import { FormField } from '../shared/form-field.tsx';
import { Checkbox } from '../shared/checkbox.tsx';
import { Button } from '../shared/button.tsx';
import { colors } from '../../theme/index.ts';

interface CopyCollectionModalProps {
  sourceName: string;
  sourceDatabase: string;
  sourceConnection: string;
  onClose: () => void;
}

type FocusField =
  | 'targetServer'
  | 'targetDatabase'
  | 'dropExisting'
  | 'includeIndexes'
  | 'newName'
  | 'filterQuery'
  | 'limit'
  | 'cancel'
  | 'copy';

export function CopyCollectionModal({
  sourceName,
  sourceDatabase,
  sourceConnection,
  onClose,
}: CopyCollectionModalProps) {
  const { state } = useApp();

  const availableTargets = state.connections.filter((c) => c.status === 'connected');

  const [targetServerIndex, setTargetServerIndex] = useState(0);
  const [targetDatabase, setTargetDatabase] = useState('');
  const [dropExisting, setDropExisting] = useState(false);
  const [includeIndexes, setIncludeIndexes] = useState(true);
  const [newName, setNewName] = useState('');
  const [filterQuery, setFilterQuery] = useState('');
  const [limit, setLimit] = useState('');
  const [focusedField, setFocusedField] = useState<FocusField>('targetServer');

  const fieldOrder: FocusField[] = [
    'targetServer',
    'targetDatabase',
    'dropExisting',
    'includeIndexes',
    'newName',
    'filterQuery',
    'limit',
    'cancel',
    'copy',
  ];

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

    if (focusedField === 'targetServer') {
      if (key.name === 'arrow-up' || key.name === 'k') {
        setTargetServerIndex((i) => Math.max(0, i - 1));
      }
      if (key.name === 'arrow-down' || key.name === 'j') {
        setTargetServerIndex((i) => Math.min(availableTargets.length - 1, i + 1));
      }
    }
  });

  const handleCopy = () => {
    // In a real app, this would initiate the copy operation
    onClose();
  };

  return (
    <Modal title="Copy Collection" width={65} height={28}>
      <scrollbox scrollY flexGrow={1}>
        <box flexDirection="column" gap={1}>
          <box flexDirection="column" marginBottom={1}>
            <text fg={colors.muted}>
              Source: {sourceDatabase}.{sourceName}
            </text>
            <text fg={colors.muted}>Connection: {sourceConnection}</text>
          </box>

          <FormField label="Target Server">
            <box
              border={focusedField === 'targetServer'}
              borderColor={colors.borderFocused}
              flexDirection="column"
              maxHeight={4}
            >
              {availableTargets.map((server, idx) => (
                <box
                  key={server.id}
                  backgroundColor={idx === targetServerIndex ? colors.selected : undefined}
                >
                  <text fg={idx === targetServerIndex ? colors.selectedText : colors.foreground}>
                    {idx === targetServerIndex ? '> ' : '  '}
                    {server.alias}
                  </text>
                </box>
              ))}
            </box>
          </FormField>

          <FormField label="Target Database">
            <input
              placeholder="database_name"
              value={targetDatabase}
              focused={focusedField === 'targetDatabase'}
              onInput={setTargetDatabase}
              backgroundColor={colors.background}
              focusedBackgroundColor={colors.border}
              textColor={colors.foreground}
            />
          </FormField>

          <box flexDirection="column" gap={0}>
            <text fg={colors.muted}>Options</text>
            <Checkbox
              label="Drop target collection if exists"
              checked={dropExisting}
              focused={focusedField === 'dropExisting'}
              onChange={setDropExisting}
            />
            <Checkbox
              label="Include indexes"
              checked={includeIndexes}
              focused={focusedField === 'includeIndexes'}
              onChange={setIncludeIndexes}
            />
          </box>

          <FormField label="New Name (optional)">
            <input
              placeholder={sourceName}
              value={newName}
              focused={focusedField === 'newName'}
              onInput={setNewName}
              backgroundColor={colors.background}
              focusedBackgroundColor={colors.border}
              textColor={colors.foreground}
            />
          </FormField>

          <FormField label="Filter Query (optional)">
            <input
              placeholder='{"status": "active"}'
              value={filterQuery}
              focused={focusedField === 'filterQuery'}
              onInput={setFilterQuery}
              backgroundColor={colors.background}
              focusedBackgroundColor={colors.border}
              textColor={colors.foreground}
            />
          </FormField>

          <FormField label="Limit (optional)">
            <input
              placeholder="1000"
              value={limit}
              focused={focusedField === 'limit'}
              onInput={setLimit}
              backgroundColor={colors.background}
              focusedBackgroundColor={colors.border}
              textColor={colors.foreground}
            />
          </FormField>

          <box flexDirection="row" justifyContent="flex-end" gap={2} marginTop={1}>
            <Button label="Cancel" focused={focusedField === 'cancel'} onPress={onClose} />
            <Button
              label="Copy"
              focused={focusedField === 'copy'}
              variant="primary"
              onPress={handleCopy}
            />
          </box>
        </box>
      </scrollbox>
    </Modal>
  );
}
