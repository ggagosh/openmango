import { useState } from 'react';
import { useKeyboard } from '@opentui/react';
import { useApp } from '../../context/app-context.tsx';
import { Modal } from '../shared/modal.tsx';
import { FormField } from '../shared/form-field.tsx';
import { Checkbox } from '../shared/checkbox.tsx';
import { Button } from '../shared/button.tsx';
import { colors } from '../../theme/index.ts';

interface CopyDatabaseModalProps {
  sourceName: string;
  sourceConnection: string;
  onClose: () => void;
}

type FocusField =
  | 'targetServer'
  | 'dropExisting'
  | 'includeIndexes'
  | 'collections'
  | 'cancel'
  | 'copy';

export function CopyDatabaseModal({
  sourceName,
  sourceConnection,
  onClose,
}: CopyDatabaseModalProps) {
  const { state } = useApp();

  // Get available target servers (exclude source)
  const availableTargets = state.connections.filter(
    (c) => c.status === 'connected' && c.id !== sourceConnection
  );

  const [targetServerIndex, setTargetServerIndex] = useState(0);
  const [dropExisting, setDropExisting] = useState(false);
  const [includeIndexes, setIncludeIndexes] = useState(true);
  const [selectedCollections, setSelectedCollections] = useState<Set<string>>(new Set());
  const [focusedField, setFocusedField] = useState<FocusField>('targetServer');

  // Mock collections list
  const collections = ['users', 'orders', 'products', 'sessions', 'logs'];

  const fieldOrder: FocusField[] = [
    'targetServer',
    'dropExisting',
    'includeIndexes',
    'collections',
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

    // Handle target server selection
    if (focusedField === 'targetServer') {
      if (key.name === 'arrow-up' || key.name === 'k') {
        setTargetServerIndex((i) => Math.max(0, i - 1));
      }
      if (key.name === 'arrow-down' || key.name === 'j') {
        setTargetServerIndex((i) => Math.min(availableTargets.length - 1, i + 1));
      }
    }

    // Handle collection multi-select (simplified)
    if (focusedField === 'collections') {
      if (key.name === 'space') {
        // Toggle all for simplicity
        if (selectedCollections.size === collections.length) {
          setSelectedCollections(new Set());
        } else {
          setSelectedCollections(new Set(collections));
        }
      }
    }
  });

  const handleCopy = () => {
    // In a real app, this would initiate the copy operation
    onClose();
  };

  return (
    <Modal title="Copy Database" width={60} height={22}>
      <box flexDirection="column" gap={1}>
        <box flexDirection="column" marginBottom={1}>
          <text fg={colors.muted}>
            Source: {sourceName} on {sourceConnection}
          </text>
        </box>

        <FormField label="Target Server">
          <box
            border={focusedField === 'targetServer'}
            borderColor={colors.borderFocused}
            flexDirection="column"
          >
            {availableTargets.length === 0 ? (
              <text fg={colors.statusError}>No other connected servers available</text>
            ) : (
              availableTargets.map((server, idx) => (
                <box
                  key={server.id}
                  backgroundColor={idx === targetServerIndex ? colors.selected : undefined}
                >
                  <text fg={idx === targetServerIndex ? colors.selectedText : colors.foreground}>
                    {idx === targetServerIndex ? '> ' : '  '}
                    {server.alias}
                  </text>
                </box>
              ))
            )}
          </box>
        </FormField>

        <box flexDirection="column" gap={0}>
          <text fg={colors.muted}>Options</text>
          <Checkbox
            label="Drop target database if exists"
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

        <box flexDirection="column">
          <text fg={colors.muted}>
            Collections ({selectedCollections.size}/{collections.length} selected)
          </text>
          <box
            border={focusedField === 'collections'}
            borderColor={colors.borderFocused}
            padding={0}
          >
            <text fg={colors.foreground}>
              {selectedCollections.size === collections.length
                ? 'All collections selected'
                : selectedCollections.size === 0
                  ? 'Press Space to select all'
                  : `${selectedCollections.size} collections selected`}
            </text>
          </box>
        </box>

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
    </Modal>
  );
}
