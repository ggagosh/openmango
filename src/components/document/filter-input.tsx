import { useState, useCallback } from 'react';
import { useKeyboard } from '@opentui/react';
import { colors } from '../../theme/index.ts';
import { getFilterSuggestions } from '../../services/field-index.ts';

interface FilterInputProps {
  value: string;
  fields: string[];
  focused: boolean;
  onApply: (filter: string) => void;
}

interface Suggestion {
  name: string;
  desc?: string;
}

export function FilterInput({ value, fields, focused, onApply }: FilterInputProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [editValue, setEditValue] = useState(value);
  const [cursorPosition, setCursorPosition] = useState(0);
  const [suggestions, setSuggestions] = useState<Suggestion[]>([]);
  const [selectedSuggestion, setSelectedSuggestion] = useState(0);

  const updateSuggestions = useCallback(
    (text: string, cursor: number) => {
      const newSuggestions = getFilterSuggestions(text, fields, cursor);
      setSuggestions(newSuggestions);
      setSelectedSuggestion(0);
    },
    [fields]
  );

  const startEditing = useCallback(() => {
    setIsEditing(true);
    const initialValue = value || '{ }';
    setEditValue(initialValue);
    // Position cursor after "{ " (position 2) for empty filter, else at end
    const cursorPos = initialValue === '{ }' ? 2 : initialValue.length;
    setCursorPosition(cursorPos);
    updateSuggestions(initialValue, cursorPos);
  }, [value, updateSuggestions]);

  const cancelEditing = useCallback(() => {
    setIsEditing(false);
    setEditValue(value);
    setSuggestions([]);
  }, [value]);

  const applyFilter = useCallback(() => {
    setIsEditing(false);
    setSuggestions([]);
    onApply(editValue);
  }, [editValue, onApply]);

  const acceptSuggestion = useCallback(() => {
    if (suggestions.length === 0) return false;

    const suggestion = suggestions[selectedSuggestion];
    if (!suggestion) return false;

    // Find where to insert the suggestion
    const afterCursor = editValue.slice(cursorPosition);

    // Find the start of the current word/partial
    let insertStart = cursorPosition;
    for (let i = cursorPosition - 1; i >= 0; i--) {
      const char = editValue[i];
      if (char === '"' || char === '{' || char === ':' || char === ' ' || char === ',') {
        insertStart = i + 1;
        break;
      }
      if (i === 0) insertStart = 0;
    }

    // Build new value with suggestion
    const prefix = editValue.slice(0, insertStart);
    const isField = !suggestion.name.startsWith('$');
    const insertText = isField ? `"${suggestion.name}"` : `"${suggestion.name}"`;

    const newValue = prefix + insertText + afterCursor;
    const newCursor = prefix.length + insertText.length;

    setEditValue(newValue);
    setCursorPosition(newCursor);
    setSuggestions([]);

    return true;
  }, [suggestions, selectedSuggestion, editValue, cursorPosition]);

  useKeyboard((key) => {
    if (!focused) return;

    // View mode - `/` to start editing
    if (!isEditing) {
      if (key.name === '/') {
        startEditing();
      }
      return;
    }

    // Edit mode
    if (key.name === 'escape') {
      if (suggestions.length > 0) {
        setSuggestions([]);
      } else {
        cancelEditing();
      }
      return;
    }

    if (key.name === 'return') {
      if (suggestions.length > 0) {
        acceptSuggestion();
      } else {
        applyFilter();
      }
      return;
    }

    if (key.name === 'tab') {
      if (acceptSuggestion()) return;
    }

    // Navigate suggestions
    if (suggestions.length > 0) {
      if (key.name === 'down' || key.name === 'j') {
        setSelectedSuggestion((prev) => Math.min(prev + 1, suggestions.length - 1));
        return;
      }
      if (key.name === 'up' || key.name === 'k') {
        setSelectedSuggestion((prev) => Math.max(prev - 1, 0));
        return;
      }
    }

    // Text input
    if (key.name === 'backspace') {
      if (cursorPosition > 0) {
        const newValue = editValue.slice(0, cursorPosition - 1) + editValue.slice(cursorPosition);
        setEditValue(newValue);
        setCursorPosition(cursorPosition - 1);
        updateSuggestions(newValue, cursorPosition - 1);
      }
      return;
    }

    if (key.name === 'left') {
      setCursorPosition((prev) => Math.max(prev - 1, 0));
      return;
    }

    if (key.name === 'right') {
      setCursorPosition((prev) => Math.min(prev + 1, editValue.length));
      return;
    }

    // Character input
    if (key.sequence && key.sequence.length === 1 && !key.ctrl && !key.meta) {
      const newValue =
        editValue.slice(0, cursorPosition) + key.sequence + editValue.slice(cursorPosition);
      setEditValue(newValue);
      setCursorPosition(cursorPosition + 1);
      updateSuggestions(newValue, cursorPosition + 1);
    }
  });

  // View mode
  if (!isEditing) {
    return (
      <box flexDirection="row">
        <text fg={colors.muted}>Filter: </text>
        <text fg={colors.foreground}>{value || '{ }'}</text>
        {focused && (
          <text fg={colors.muted}>
            {' '}
            <span fg={colors.primary}>/</span>=edit
          </text>
        )}
      </box>
    );
  }

  // Edit mode
  const beforeCursor = editValue.slice(0, cursorPosition);
  const afterCursor = editValue.slice(cursorPosition);

  return (
    <box flexDirection="column">
      <box flexDirection="row">
        <text fg={colors.muted}>Filter: </text>
        <text fg={colors.warning}>{beforeCursor}</text>
        <box backgroundColor={colors.primary}>
          <text fg={colors.background}>{afterCursor[0] || ' '}</text>
        </box>
        <text fg={colors.warning}>{afterCursor.slice(1)}</text>
      </box>

      {/* Suggestions dropdown */}
      {suggestions.length > 0 && (
        <box flexDirection="column" paddingLeft={8}>
          {suggestions.map((suggestion, index) => (
            <box
              key={suggestion.name}
              flexDirection="row"
              backgroundColor={index === selectedSuggestion ? colors.selected : undefined}
            >
              <text fg={index === selectedSuggestion ? colors.foreground : colors.muted}>
                {index === selectedSuggestion ? '> ' : '  '}
                {suggestion.name}
              </text>
              {suggestion.desc && <text fg={colors.muted}> - {suggestion.desc}</text>}
            </box>
          ))}
        </box>
      )}
    </box>
  );
}
