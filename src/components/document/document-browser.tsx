import { useCallback, useEffect, useRef, useState } from 'react';
import { useKeyboard } from '@opentui/react';
import { type Db, ObjectId } from 'mongodb';
import { DocumentProvider, useDocument } from '../../context/document-context.tsx';
import { useFocus } from '../../context/focus-context.tsx';
import { findDocuments, updateDocument } from '../../services/document.ts';
import { getCollectionStats } from '../../services/collection.ts';
import type { CollectionStats } from '../../types/index.ts';
import { colors } from '../../theme/index.ts';
import { DocumentList } from './document-list.tsx';
import { flattenVisiblePaths, getValueAtPath, isExpandable } from './json-tree.tsx';

interface DocumentBrowserProps {
  db: Db;
  collectionName: string;
  focused: boolean;
}

function DocumentBrowserContent({ db, collectionName, focused }: DocumentBrowserProps) {
  const { state, dispatch, selectNext, selectPrev, toggleSelected, nextPage, prevPage } =
    useDocument();
  const { setActiveZone } = useFocus();
  const [commandBuffer, setCommandBuffer] = useState('');
  const [isTypingCommand, setIsTypingCommand] = useState(false);
  const [stats, setStats] = useState<CollectionStats | null>(null);
  const loadedCollectionRef = useRef<string | null>(null);
  const isInitialLoadRef = useRef(true);

  const {
    documents,
    totalCount,
    currentPage,
    pageSize,
    selectedIndex,
    isLoading,
    error,
    treeNavigationActive,
    treeExpandedPaths,
    selectedPath,
    editingPath,
    editingValue,
    editingValueType,
    hasUnsavedChanges,
    editingId,
  } = state;

  const totalPages = Math.ceil(totalCount / pageSize);

  const loadDocuments = useCallback(async () => {
    dispatch({ type: 'SET_LOADING', payload: true });
    try {
      const result = await findDocuments(db, collectionName, {
        filter: state.filter || undefined,
        limit: pageSize,
        skip: (currentPage - 1) * pageSize,
      });
      dispatch({ type: 'SET_DOCUMENTS', payload: result });
    } catch (e) {
      dispatch({
        type: 'SET_ERROR',
        payload: e instanceof Error ? e.message : 'Failed to load documents',
      });
    }
  }, [db, collectionName, state.filter, currentPage, pageSize, dispatch]);

  const loadStats = useCallback(async () => {
    try {
      const collStats = await getCollectionStats(db, collectionName);
      setStats(collStats);
    } catch {
      setStats(null);
    }
  }, [db, collectionName]);

  useEffect(() => {
    if (loadedCollectionRef.current !== collectionName) {
      loadedCollectionRef.current = collectionName;
      isInitialLoadRef.current = true;
      dispatch({ type: 'RESET' });
      setStats(null);
      void loadDocuments();
      void loadStats();
    }
  }, [collectionName, dispatch, loadDocuments, loadStats]);

  useEffect(() => {
    if (isInitialLoadRef.current) {
      isInitialLoadRef.current = false;
      return;
    }
    if (loadedCollectionRef.current === collectionName) {
      void loadDocuments();
    }
  }, [currentPage, state.filter, loadDocuments, collectionName]);

  // Get current document for tree navigation
  const currentDoc = documents[selectedIndex];
  const docToDisplay = state.pendingDocument ?? currentDoc;

  const handleEnterTreeNavigation = useCallback(() => {
    const doc = documents[selectedIndex];
    if (!doc) return;

    const docId = doc._id.toString();

    if (hasUnsavedChanges && editingId && editingId !== docId) {
      return;
    }

    // Auto-expand document if not expanded
    if (!state.expandedIds.has(docId)) {
      dispatch({ type: 'TOGGLE_EXPAND', payload: docId });
    }

    // Enter tree navigation mode
    dispatch({ type: 'ENTER_TREE_NAVIGATION', payload: { docId } });
  }, [documents, selectedIndex, state.expandedIds, dispatch, hasUnsavedChanges, editingId]);

  const handleSaveDocument = useCallback(async () => {
    if (!state.pendingDocument) return;
    const targetDoc = editingId
      ? documents.find((doc) => doc._id.toString() === editingId)
      : currentDoc;
    if (!targetDoc) return;

    try {
      const success = await updateDocument(
        db,
        collectionName,
        targetDoc._id,
        state.pendingDocument
      );
      if (success) {
        dispatch({
          type: 'SAVE_DOCUMENT_SUCCESS',
          payload: { ...state.pendingDocument, _id: targetDoc._id },
        });
        void loadDocuments(); // Reload to get fresh data
      }
    } catch (e) {
      dispatch({
        type: 'SET_ERROR',
        payload: e instanceof Error ? e.message : 'Failed to save document',
      });
    }
  }, [
    db,
    collectionName,
    currentDoc,
    documents,
    editingId,
    state.pendingDocument,
    dispatch,
    loadDocuments,
  ]);

  const handleCommand = useCallback(
    (cmd: string) => {
      if (cmd === ':w') {
        void handleSaveDocument();
      } else if (cmd === ':q') {
        dispatch({ type: 'DISCARD_DOCUMENT_CHANGES' });
      } else if (cmd === ':wq') {
        void handleSaveDocument();
      }
      setCommandBuffer('');
      setIsTypingCommand(false);
    },
    [dispatch, handleSaveDocument]
  );

  // Helper to parse edit value to proper type
  const formatEditValue = (value: unknown): string => {
    if (value === null) return 'null';
    if (typeof value === 'string') return JSON.stringify(value);
    if (typeof value === 'number' || typeof value === 'boolean') return String(value);
    if (value instanceof Date) {
      return `ISODate("${value.toISOString()}")`;
    }
    if (typeof value === 'object' && value && '_bsontype' in value) {
      const bsonType = (value as { _bsontype: string })._bsontype;
      if (bsonType === 'ObjectId') {
        const id = (value as { toString(): string }).toString();
        return `ObjectId("${id}")`;
      }
      if (bsonType === 'Date') {
        const dateStr = (value as unknown as Date).toISOString();
        return `ISODate("${dateStr}")`;
      }
    }
    return JSON.stringify(value) ?? '';
  };

  const parseEditValue = (
    value: string,
    expectedType: 'string' | 'number' | 'boolean' | 'null' | 'date' | 'objectId' | 'other' | null
  ): unknown => {
    if (expectedType === 'string') {
      return value;
    }

    const trimmed = value.trim();
    if (trimmed === 'undefined') return undefined;
    if (trimmed === 'null') return null;
    if (trimmed === 'true') return true;
    if (trimmed === 'false') return false;
    if (/^-?\d+$/.test(trimmed)) return parseInt(trimmed, 10);
    if (/^-?\d+\.\d+$/.test(trimmed)) return parseFloat(trimmed);
    if (trimmed.startsWith('ObjectId("') && trimmed.endsWith('")')) {
      const hex = trimmed.slice(10, -2);
      if (ObjectId.isValid(hex)) {
        return new ObjectId(hex);
      }
    }
    if (trimmed.startsWith('ISODate("') && trimmed.endsWith('")')) {
      const iso = trimmed.slice(9, -2);
      const date = new Date(iso);
      if (!Number.isNaN(date.getTime())) {
        return date;
      }
    }
    if (
      (trimmed.startsWith('{') && trimmed.endsWith('}')) ||
      (trimmed.startsWith('[') && trimmed.endsWith(']'))
    ) {
      try {
        return JSON.parse(trimmed);
      } catch {
        // Fall through to string parsing
      }
    }
    if (trimmed.startsWith('"') && trimmed.endsWith('"')) {
      try {
        return JSON.parse(trimmed);
      } catch {
        return trimmed.slice(1, -1);
      }
    }
    return trimmed;
  };

  useKeyboard((key) => {
    if (!focused) return;

    // Command mode (typing :w, :q, etc.)
    if (isTypingCommand) {
      if (key.name === 'return') {
        handleCommand(commandBuffer);
      } else if (key.name === 'escape') {
        setCommandBuffer('');
        setIsTypingCommand(false);
      } else if (key.name === 'backspace') {
        setCommandBuffer((prev) => prev.slice(0, -1));
        if (commandBuffer.length <= 1) {
          setIsTypingCommand(false);
        }
      } else if (key.name && key.name.length === 1 && !key.ctrl && !key.meta) {
        setCommandBuffer((prev) => prev + key.name);
      }
      return;
    }

    // Field editing mode
    if (editingPath) {
      if (key.name === 'return') {
        // Apply the edit
        const newValue = parseEditValue(editingValue, editingValueType);
        dispatch({ type: 'APPLY_FIELD_EDIT', payload: { path: editingPath, newValue } });
      } else if (key.name === 'escape') {
        dispatch({ type: 'CANCEL_FIELD_EDIT' });
      } else if (key.name === 'backspace') {
        dispatch({ type: 'UPDATE_FIELD_VALUE', payload: editingValue.slice(0, -1) });
      } else if (key.name === 'space') {
        dispatch({ type: 'UPDATE_FIELD_VALUE', payload: editingValue + ' ' });
      } else if (key.name && key.name.length === 1 && !key.ctrl && !key.meta) {
        dispatch({ type: 'UPDATE_FIELD_VALUE', payload: editingValue + key.name });
      }
      return;
    }

    // Tree navigation mode
    if (treeNavigationActive && docToDisplay && selectedPath) {
      const visiblePaths = flattenVisiblePaths(docToDisplay, treeExpandedPaths);
      const currentIndex = visiblePaths.indexOf(selectedPath);

      if (key.name === 'j' || key.name === 'down') {
        const next = Math.min(currentIndex + 1, visiblePaths.length - 1);
        dispatch({ type: 'SELECT_PATH', payload: visiblePaths[next]! });
      } else if (key.name === 'k' || key.name === 'up') {
        const prev = Math.max(currentIndex - 1, 0);
        dispatch({ type: 'SELECT_PATH', payload: visiblePaths[prev]! });
      } else if (key.name === 'l' || key.name === 'right') {
        const value = getValueAtPath(docToDisplay, selectedPath);
        if (isExpandable(value) && !treeExpandedPaths.has(selectedPath)) {
          dispatch({ type: 'TOGGLE_TREE_PATH', payload: selectedPath });
        } else if (treeExpandedPaths.has(selectedPath)) {
          // Move to first child
          const childPath = visiblePaths[currentIndex + 1];
          if (childPath?.startsWith(selectedPath + '.')) {
            dispatch({ type: 'SELECT_PATH', payload: childPath });
          }
        }
      } else if (key.name === 'h' || key.name === 'left') {
        const value = getValueAtPath(docToDisplay, selectedPath);
        if (isExpandable(value) && treeExpandedPaths.has(selectedPath)) {
          dispatch({ type: 'TOGGLE_TREE_PATH', payload: selectedPath });
        } else if (selectedPath === 'root') {
          // At root level with nothing to collapse - exit tree navigation
          dispatch({ type: 'EXIT_TREE_NAVIGATION' });
        } else {
          // Move to parent
          const parts = selectedPath.split('.');
          if (parts.length > 1) {
            const parentPath = parts.slice(0, -1).join('.');
            dispatch({ type: 'SELECT_PATH', payload: parentPath });
          }
        }
      } else if (key.name === 'return') {
        const value = getValueAtPath(docToDisplay, selectedPath);
        if (isExpandable(value)) {
          dispatch({ type: 'TOGGLE_TREE_PATH', payload: selectedPath });
        }
      } else if (key.name === 'i') {
        // Start editing current field (only for primitives)
        const value = getValueAtPath(docToDisplay, selectedPath);
        if (!isExpandable(value)) {
          let valueType: 'string' | 'number' | 'boolean' | 'null' | 'date' | 'objectId' | 'other' =
            'other';
          if (value === null) {
            valueType = 'null';
          } else if (typeof value === 'string') {
            valueType = 'string';
          } else if (typeof value === 'number') {
            valueType = 'number';
          } else if (typeof value === 'boolean') {
            valueType = 'boolean';
          } else if (value instanceof Date) {
            valueType = 'date';
          } else if (typeof value === 'object' && value && '_bsontype' in value) {
            const bsonType = (value as { _bsontype: string })._bsontype;
            if (bsonType === 'ObjectId') {
              valueType = 'objectId';
            } else if (bsonType === 'Date') {
              valueType = 'date';
            }
          }

          const valueStr = valueType === 'string' ? (value as string) : formatEditValue(value);
          dispatch({
            type: 'START_FIELD_EDIT',
            payload: { path: selectedPath, value: valueStr, valueType },
          });
        }
      } else if (key.name === 'escape') {
        if (hasUnsavedChanges) {
          // Keep pending changes but exit tree navigation
        }
        dispatch({ type: 'EXIT_TREE_NAVIGATION' });
      } else if (key.name === ':') {
        setIsTypingCommand(true);
        setCommandBuffer(':');
      }
      return;
    }

    // Document list view mode
    if (key.name === 'down' || key.name === 'j') {
      selectNext();
    } else if (key.name === 'up' || key.name === 'k') {
      selectPrev();
    } else if (key.name === 'return') {
      toggleSelected();
    } else if (key.name === 'right' || key.name === 'l') {
      const doc = documents[selectedIndex];
      if (doc && !state.expandedIds.has(doc._id.toString())) {
        toggleSelected();
      } else if (doc && state.expandedIds.has(doc._id.toString())) {
        // Enter tree navigation
        handleEnterTreeNavigation();
      }
    } else if (key.name === 'left' || key.name === 'h') {
      const doc = documents[selectedIndex];
      if (doc && state.expandedIds.has(doc._id.toString())) {
        toggleSelected();
      } else {
        setActiveZone('sidebar');
      }
    } else if (key.name === 'i') {
      handleEnterTreeNavigation();
    } else if (key.sequence === '<' || (key.shift && key.name === ',')) {
      prevPage();
    } else if (key.sequence === '>' || (key.shift && key.name === '.')) {
      nextPage();
    } else if (key.name === ':' && hasUnsavedChanges) {
      setIsTypingCommand(true);
      setCommandBuffer(':');
    }
  });

  // Mode label
  let modeLabel = '[VIEW]';
  let modeColor = colors.muted;
  if (treeNavigationActive) {
    if (editingPath) {
      modeLabel = '[EDITING FIELD]';
      modeColor = colors.warning;
    } else {
      modeLabel = hasUnsavedChanges ? '[TREE *]' : '[TREE]';
      modeColor = hasUnsavedChanges ? colors.warning : colors.primary;
    }
  } else if (hasUnsavedChanges) {
    modeLabel = '[VIEW *]';
    modeColor = colors.warning;
  }

  return (
    <box flexDirection="column" flexGrow={1}>
      {/* Header */}
      <box flexDirection="row" paddingLeft={1} paddingRight={1}>
        <text fg={colors.foreground}>COLLECTION: {collectionName}</text>
        <box flexGrow={1} />
        <text fg={modeColor}>{modeLabel}</text>
      </box>

      {/* Stats row */}
      <box flexDirection="row" paddingLeft={1} paddingRight={1}>
        {stats ? (
          <text fg={colors.muted}>
            {stats.documentCount} docs | {stats.totalSize} | {stats.indexCount} indexes
          </text>
        ) : (
          <text fg={colors.muted}>Loading stats...</text>
        )}
      </box>

      {/* Filter row */}
      <box flexDirection="row" paddingLeft={1} paddingRight={1}>
        <text fg={colors.muted}>Filter: </text>
        <text fg={colors.foreground}>{state.filter || '{ }'}</text>
      </box>

      {/* Document list - takes remaining space */}
      <box flexDirection="column" flexGrow={1} paddingTop={1}>
        {isLoading ? (
          <box paddingLeft={1}>
            <text fg={colors.muted}>Loading...</text>
          </box>
        ) : error ? (
          <box paddingLeft={1}>
            <text fg={colors.statusError}>{error}</text>
          </box>
        ) : (
          <DocumentList />
        )}
      </box>

      {/* Footer - always at bottom */}
      <box flexDirection="row" paddingLeft={1} paddingRight={1}>
        {isTypingCommand ? (
          <text fg={colors.foreground}>{commandBuffer}</text>
        ) : editingPath ? (
          <text fg={colors.muted}>Enter=save Esc=cancel</text>
        ) : treeNavigationActive ? (
          <text fg={colors.muted}>
            j/k=nav h/l=collapse/expand i=edit Esc=exit :w=save :q=discard
          </text>
        ) : hasUnsavedChanges ? (
          <text fg={colors.warning}>Unsaved changes: i=resume :w=save :q=discard</text>
        ) : (
          <>
            <text fg={colors.muted}>
              {'<'}Prev Page {currentPage}/{totalPages} Next{'>'}
            </text>
            <box flexGrow={1} />
            <text fg={colors.muted}>i/→=tree</text>
          </>
        )}
      </box>
    </box>
  );
}

export function DocumentBrowser(props: DocumentBrowserProps) {
  return (
    <DocumentProvider>
      <DocumentBrowserContent {...props} />
    </DocumentProvider>
  );
}
