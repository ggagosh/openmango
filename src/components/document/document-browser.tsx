import { useCallback, useEffect, useRef, useState } from 'react';
import { useKeyboard } from '@opentui/react';
import type { Db } from 'mongodb';
import { DocumentProvider, useDocument } from '../../context/document-context.tsx';
import { useFocus } from '../../context/focus-context.tsx';
import { findDocuments, updateDocument } from '../../services/document.ts';
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
    hasUnsavedChanges,
  } = state;

  const totalPages = Math.ceil(totalCount / pageSize);
  const startIndex = (currentPage - 1) * pageSize + 1;
  const endIndex = Math.min(currentPage * pageSize, totalCount);

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

  useEffect(() => {
    if (loadedCollectionRef.current !== collectionName) {
      loadedCollectionRef.current = collectionName;
      isInitialLoadRef.current = true;
      dispatch({ type: 'RESET' });
      void loadDocuments();
    }
  }, [collectionName, dispatch, loadDocuments]);

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

    // Auto-expand document if not expanded
    if (!state.expandedIds.has(docId)) {
      dispatch({ type: 'TOGGLE_EXPAND', payload: docId });
    }

    // Enter tree navigation mode
    dispatch({ type: 'ENTER_TREE_NAVIGATION', payload: { docId } });
  }, [documents, selectedIndex, state.expandedIds, dispatch]);

  const handleSaveDocument = useCallback(async () => {
    if (!state.pendingDocument || !currentDoc) return;

    try {
      const success = await updateDocument(
        db,
        collectionName,
        currentDoc._id,
        state.pendingDocument
      );
      if (success) {
        dispatch({
          type: 'SAVE_DOCUMENT_SUCCESS',
          payload: { ...state.pendingDocument, _id: currentDoc._id },
        });
        void loadDocuments(); // Reload to get fresh data
      }
    } catch (e) {
      dispatch({
        type: 'SET_ERROR',
        payload: e instanceof Error ? e.message : 'Failed to save document',
      });
    }
  }, [db, collectionName, currentDoc, state.pendingDocument, dispatch, loadDocuments]);

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
  const parseEditValue = (value: string): unknown => {
    const trimmed = value.trim();
    if (trimmed === 'null') return null;
    if (trimmed === 'true') return true;
    if (trimmed === 'false') return false;
    if (/^-?\d+$/.test(trimmed)) return parseInt(trimmed, 10);
    if (/^-?\d+\.\d+$/.test(trimmed)) return parseFloat(trimmed);
    // Remove quotes for strings
    if (trimmed.startsWith('"') && trimmed.endsWith('"')) {
      return trimmed.slice(1, -1);
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
        const newValue = parseEditValue(editingValue);
        dispatch({ type: 'APPLY_FIELD_EDIT', payload: { path: editingPath, newValue } });
      } else if (key.name === 'escape') {
        dispatch({ type: 'CANCEL_FIELD_EDIT' });
      } else if (key.name === 'backspace') {
        dispatch({ type: 'UPDATE_FIELD_VALUE', payload: editingValue.slice(0, -1) });
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
          let valueStr: string;
          if (value === null) {
            valueStr = 'null';
          } else if (typeof value === 'string') {
            valueStr = `"${value}"`;
          } else if (typeof value === 'number' || typeof value === 'boolean') {
            valueStr = String(value);
          } else {
            valueStr = JSON.stringify(value) ?? '';
          }
          dispatch({ type: 'START_FIELD_EDIT', payload: { path: selectedPath, value: valueStr } });
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
    } else if (key.name === '[') {
      prevPage();
    } else if (key.name === ']') {
      nextPage();
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
  }

  return (
    <box flexDirection="column" flexGrow={1}>
      {/* Header */}
      <box flexDirection="row" paddingLeft={1} paddingRight={1}>
        <text fg={colors.foreground}>COLLECTION: {collectionName}</text>
        <box flexGrow={1} />
        <text fg={modeColor}>{modeLabel}</text>
        <text fg={colors.muted}>
          {' '}
          [{startIndex}-{endIndex} of {totalCount}]
        </text>
      </box>

      {/* Filter row */}
      <box flexDirection="row" paddingLeft={1} paddingRight={1} paddingTop={1}>
        <text fg={colors.muted}>Filter: </text>
        <text fg={colors.foreground}>{state.filter || '{ }'}</text>
      </box>

      {/* Document list */}
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

      {/* Footer */}
      <box flexDirection="row" paddingLeft={1} paddingRight={1} paddingTop={1}>
        {isTypingCommand ? (
          <text fg={colors.foreground}>{commandBuffer}</text>
        ) : editingPath ? (
          <text fg={colors.muted}>Enter=save Esc=cancel</text>
        ) : treeNavigationActive ? (
          <text fg={colors.muted}>
            j/k=nav h/l=collapse/expand i=edit Esc=exit :w=save :q=discard
          </text>
        ) : (
          <>
            <text fg={colors.muted}>
              [◀ Prev] Page {currentPage} of {totalPages} [Next ▶]
            </text>
            <box flexGrow={1} />
            <text fg={colors.muted}>i/→=enter tree n=new d=delete</text>
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
