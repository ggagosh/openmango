import { useCallback, useEffect, useRef, useState } from 'react';
import { useKeyboard } from '@opentui/react';
import type { Db } from 'mongodb';
import { DocumentProvider, useDocument } from '../../context/document-context.tsx';
import { useFocus } from '../../context/focus-context.tsx';
import { findDocuments } from '../../services/document.ts';
import { colors } from '../../theme/index.ts';
import { formatDocument } from '../../utils/json-formatter.ts';
import { DocumentList } from './document-list.tsx';

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

  const { documents, totalCount, currentPage, pageSize, mode, selectedIndex, isLoading, error } =
    state;

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
    // Only reset and initial load when collection changes
    if (loadedCollectionRef.current !== collectionName) {
      loadedCollectionRef.current = collectionName;
      isInitialLoadRef.current = true;
      dispatch({ type: 'RESET' });
      void loadDocuments();
    }
  }, [collectionName, dispatch, loadDocuments]);

  useEffect(() => {
    // Reload when page or filter changes (but skip initial load - already handled above)
    if (isInitialLoadRef.current) {
      isInitialLoadRef.current = false;
      return;
    }
    if (loadedCollectionRef.current === collectionName) {
      void loadDocuments();
    }
  }, [currentPage, state.filter, loadDocuments, collectionName]);

  const handleEnterEditMode = useCallback(() => {
    const doc = documents[selectedIndex];
    if (!doc) return;

    const docId = doc._id.toString();

    // Auto-expand if not expanded
    if (!state.expandedIds.has(docId)) {
      dispatch({ type: 'TOGGLE_EXPAND', payload: docId });
    }

    // Enter edit mode
    dispatch({
      type: 'ENTER_EDIT_MODE',
      payload: {
        id: docId,
        content: formatDocument(doc),
        original: doc,
      },
    });
  }, [documents, selectedIndex, state.expandedIds, dispatch]);

  const handleCommand = useCallback(
    (cmd: string) => {
      if (cmd === ':w') {
        // TODO: Save changes
        dispatch({ type: 'SAVE_SUCCESS' });
      } else if (cmd === ':q') {
        dispatch({ type: 'DISCARD_CHANGES' });
      } else if (cmd === ':wq') {
        // TODO: Save then exit
        dispatch({ type: 'SAVE_SUCCESS' });
      }
      setCommandBuffer('');
      setIsTypingCommand(false);
    },
    [dispatch]
  );

  useKeyboard((key) => {
    if (!focused) return;

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

    if (mode === 'edit') {
      if (key.name === 'escape') {
        dispatch({ type: 'EXIT_EDIT_MODE' });
      } else if (key.name === ':') {
        setIsTypingCommand(true);
        setCommandBuffer(':');
      }
      return;
    }

    // VIEW mode keys
    if (key.name === 'down' || key.name === 'j') {
      selectNext();
    } else if (key.name === 'up' || key.name === 'k') {
      selectPrev();
    } else if (key.name === 'return') {
      toggleSelected();
    } else if (key.name === 'right' || key.name === 'l') {
      // Expand selected document (drill into hierarchy)
      const doc = documents[selectedIndex];
      if (doc && !state.expandedIds.has(doc._id.toString())) {
        toggleSelected();
      }
    } else if (key.name === 'left' || key.name === 'h') {
      // Collapse selected document OR go back to sidebar
      const doc = documents[selectedIndex];
      if (doc && state.expandedIds.has(doc._id.toString())) {
        toggleSelected(); // Collapse
      } else {
        setActiveZone('sidebar'); // Go back to sidebar
      }
    } else if (key.name === 'i') {
      handleEnterEditMode();
    } else if (key.name === '[') {
      prevPage();
    } else if (key.name === ']') {
      nextPage();
    }
  });

  const modeLabel = mode === 'edit' ? '[EDIT MODE]' : '[VIEW MODE]';
  const modeColor = mode === 'edit' ? colors.warning : colors.muted;

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
        ) : mode === 'edit' ? (
          <text fg={colors.muted}>Esc=exit edit :w=save :q=discard :wq=save & exit</text>
        ) : (
          <>
            <text fg={colors.muted}>
              [◀ Prev] Page {currentPage} of {totalPages} [Next ▶]
            </text>
            <box flexGrow={1} />
            <text fg={colors.muted}>i=edit n=new d=delete</text>
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
