import { useState, useEffect } from 'react';
import { useKeyboard } from '@opentui/react';
import { useApp } from '../../context/app-context.tsx';
import { Modal } from '../shared/modal.tsx';
import { FormField } from '../shared/form-field.tsx';
import { Button } from '../shared/button.tsx';
import { colors, icons } from '../../theme/index.ts';
import { parseConnectionString } from '../../utils/connection-string-parser.ts';
import { mongoService } from '../../services/mongodb.ts';
import { updateConnection as persistConnection } from '../../services/persistence.ts';
import type {
  ValidationResult,
  TestConnectionResult,
  ParsedConnectionString,
} from '../../types/index.ts';

interface EditConnectionModalProps {
  connectionId: string;
  onClose: () => void;
}

type FocusField = 'url' | 'alias' | 'cancel' | 'test' | 'save';
type TestState = 'idle' | 'testing' | 'success' | 'error';

function getDefaultAlias(parsed: ParsedConnectionString): string {
  if (parsed.username) {
    return `${parsed.username}@${parsed.displayHost}`;
  }
  return parsed.displayHost;
}

function getErrorDescription(errorType?: string): string {
  switch (errorType) {
    case 'network':
      return 'Connection refused';
    case 'auth':
      return 'Authentication failed';
    case 'timeout':
      return 'Connection timed out';
    case 'dns':
      return 'DNS lookup failed';
    case 'ssl':
      return 'SSL/TLS error';
    default:
      return 'Connection failed';
  }
}

export function EditConnectionModal({ connectionId, onClose }: EditConnectionModalProps) {
  const { state, dispatch } = useApp();
  const connection = state.connections.find((c) => c.id === connectionId);

  const [url, setUrl] = useState(connection?.url ?? 'mongodb://');
  const [alias, setAlias] = useState(connection?.alias ?? '');
  const [focusedField, setFocusedField] = useState<FocusField>('url');

  // Validation state
  const [validation, setValidation] = useState<ValidationResult>({ isValid: false });
  const [formatError, setFormatError] = useState<string>();

  // Test connection state
  const [testState, setTestState] = useState<TestState>('idle');
  const [testResult, setTestResult] = useState<TestConnectionResult>();

  // Track if URL has changed from original
  const [urlChanged, setUrlChanged] = useState(false);

  const fieldOrder: FocusField[] = ['url', 'alias', 'cancel', 'test', 'save'];

  const canTest = validation.isValid && testState !== 'testing';
  const canSave = validation.isValid;

  // Validate on URL change
  useEffect(() => {
    if (url && url !== 'mongodb://' && url !== 'mongodb+srv://') {
      const result = parseConnectionString(url);
      setValidation(result);
      setFormatError(result.isValid ? undefined : result.error);

      // Check if URL changed
      setUrlChanged(url !== connection?.url);

      // Reset test state when URL changes
      if (url !== connection?.url) {
        setTestState('idle');
        setTestResult(undefined);
      }
    } else {
      setValidation({ isValid: false });
      setFormatError(undefined);
    }
  }, [url, connection?.url]);

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

  const handleTest = async () => {
    if (!validation.isValid) {
      setFormatError(validation.error ?? 'Invalid connection string');
      return;
    }

    setTestState('testing');
    setTestResult(undefined);

    const result = await mongoService.testConnection(url);
    setTestResult(result);
    setTestState(result.success ? 'success' : 'error');
  };

  const handleSave = async () => {
    if (!validation.isValid || !connection) {
      setFormatError(validation.error ?? 'Invalid connection string');
      return;
    }

    const updates = {
      url,
      alias: alias || getDefaultAlias(validation.parsed!),
      parsed: validation.parsed,
    };

    // Persist to disk
    await persistConnection(connectionId, updates);

    // Update state
    dispatch({ type: 'UPDATE_CONNECTION', payload: { id: connectionId, ...updates } });
    onClose();
  };

  if (!connection) {
    return (
      <Modal title="Edit Connection" width={70} height={10}>
        <text fg={colors.statusError}>Connection not found</text>
      </Modal>
    );
  }

  return (
    <Modal title="Edit Connection" width={70} height={22}>
      <box flexDirection="column" gap={1}>
        {/* Connection URL Input */}
        <FormField label="Connection URL" required>
          <input
            placeholder="mongodb://localhost:27017 or mongodb+srv://..."
            value={url}
            focused={focusedField === 'url'}
            onInput={(value) => {
              setUrl(value);
              setFormatError(undefined);
            }}
            onSubmit={() => void handleSave()}
            backgroundColor={colors.background}
            focusedBackgroundColor={colors.border}
            textColor={colors.foreground}
          />
        </FormField>
        {formatError && <text fg={colors.statusError}>{formatError}</text>}

        {/* Parsed Info Display */}
        {validation.isValid && validation.parsed && (
          <box flexDirection="column" marginBottom={1}>
            <text fg={colors.muted}>
              {validation.parsed.isSRV ? 'SRV' : 'Standard'} connection
              {validation.parsed.username && ` as ${validation.parsed.username}`}
              {validation.parsed.isReplicaSet && ' (Replica Set)'}
            </text>
            {validation.parsed.options.replicaSet && (
              <text fg={colors.muted}>Replica Set: {validation.parsed.options.replicaSet}</text>
            )}
            {validation.parsed.options.tls && <text fg={colors.muted}>TLS enabled</text>}
          </box>
        )}

        {/* Alias Input */}
        <FormField label="Alias (optional)">
          <input
            placeholder={validation.parsed ? getDefaultAlias(validation.parsed) : 'My Database'}
            value={alias}
            focused={focusedField === 'alias'}
            onInput={setAlias}
            onSubmit={() => void handleSave()}
            backgroundColor={colors.background}
            focusedBackgroundColor={colors.border}
            textColor={colors.foreground}
          />
        </FormField>

        {/* URL Changed Warning */}
        {urlChanged && testState === 'idle' && (
          <text fg={colors.warning}>URL changed - consider testing before saving</text>
        )}

        {/* Test Result Display */}
        {testState !== 'idle' && (
          <box flexDirection="column" marginTop={1}>
            {testState === 'testing' && (
              <text fg={colors.statusConnecting}>{icons.connecting} Testing connection...</text>
            )}
            {testState === 'success' && testResult && (
              <box flexDirection="column">
                <text fg={colors.statusConnected}>
                  {icons.connected} Connected ({testResult.latencyMs}ms)
                </text>
                {testResult.serverInfo?.version && (
                  <text fg={colors.muted}> MongoDB v{testResult.serverInfo.version}</text>
                )}
              </box>
            )}
            {testState === 'error' && testResult && (
              <box flexDirection="column">
                <text fg={colors.statusError}>
                  {icons.error} {getErrorDescription(testResult.errorType)}
                </text>
                <text fg={colors.muted}> {testResult.error}</text>
              </box>
            )}
          </box>
        )}

        {/* Action Buttons */}
        <box flexDirection="row" justifyContent="flex-end" gap={2} marginTop={1}>
          <Button label="Cancel" focused={focusedField === 'cancel'} onPress={onClose} />
          <Button
            label={testState === 'testing' ? 'Testing...' : 'Test'}
            focused={focusedField === 'test'}
            variant="secondary"
            onPress={() => void handleTest()}
            disabled={!canTest}
          />
          <Button
            label="Save"
            focused={focusedField === 'save'}
            variant="primary"
            onPress={() => void handleSave()}
            disabled={!canSave}
          />
        </box>
      </box>
    </Modal>
  );
}
