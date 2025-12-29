import { type ReactNode, createContext, useCallback, useContext, useReducer } from 'react';
import type { Document, WithId } from 'mongodb';
import type { EditorMode } from '../types/index.ts';

interface DocumentState {
  mode: EditorMode;
  documents: WithId<Document>[];
  totalCount: number;
  currentPage: number;
  pageSize: number;
  filter: string;
  selectedIndex: number;
  expandedIds: Set<string>;
  // Tree navigation state
  treeNavigationActive: boolean;
  treeExpandedPaths: Set<string>;
  selectedPath: string | null;
  // Field editing state
  editingPath: string | null;
  editingValue: string;
  // Pending changes (staged, not saved to MongoDB yet)
  pendingDocument: Document | null;
  hasUnsavedChanges: boolean;
  // Legacy (for backwards compat)
  editingId: string | null;
  editBuffer: string;
  originalDoc: Document | null;
  // Loading state
  isLoading: boolean;
  error: string | null;
}

type DocumentAction =
  | { type: 'SET_DOCUMENTS'; payload: { documents: WithId<Document>[]; totalCount: number } }
  | { type: 'SET_PAGE'; payload: number }
  | { type: 'SET_FILTER'; payload: string }
  | { type: 'SELECT_INDEX'; payload: number }
  | { type: 'TOGGLE_EXPAND'; payload: string }
  // Tree navigation
  | { type: 'ENTER_TREE_NAVIGATION'; payload: { docId: string } }
  | { type: 'EXIT_TREE_NAVIGATION' }
  | { type: 'SELECT_PATH'; payload: string }
  | { type: 'TOGGLE_TREE_PATH'; payload: string }
  // Field editing
  | { type: 'START_FIELD_EDIT'; payload: { path: string; value: string } }
  | { type: 'UPDATE_FIELD_VALUE'; payload: string }
  | { type: 'APPLY_FIELD_EDIT'; payload: { path: string; newValue: unknown } }
  | { type: 'CANCEL_FIELD_EDIT' }
  // Document changes
  | { type: 'SET_PENDING_DOCUMENT'; payload: Document }
  | { type: 'SAVE_DOCUMENT_SUCCESS'; payload: WithId<Document> }
  | { type: 'DISCARD_DOCUMENT_CHANGES' }
  // Legacy edit mode
  | { type: 'ENTER_EDIT_MODE'; payload: { id: string; content: string; original: Document } }
  | { type: 'EXIT_EDIT_MODE' }
  | { type: 'UPDATE_EDIT_BUFFER'; payload: string }
  | { type: 'SAVE_SUCCESS' }
  | { type: 'DISCARD_CHANGES' }
  // Loading
  | { type: 'SET_LOADING'; payload: boolean }
  | { type: 'SET_ERROR'; payload: string | null }
  | { type: 'RESET' };

const defaultState: DocumentState = {
  mode: 'view',
  documents: [],
  totalCount: 0,
  currentPage: 1,
  pageSize: 25,
  filter: '',
  selectedIndex: 0,
  expandedIds: new Set(),
  // Tree navigation
  treeNavigationActive: false,
  treeExpandedPaths: new Set(['root']),
  selectedPath: 'root',
  // Field editing
  editingPath: null,
  editingValue: '',
  // Pending changes
  pendingDocument: null,
  hasUnsavedChanges: false,
  // Legacy
  editingId: null,
  editBuffer: '',
  originalDoc: null,
  // Loading
  isLoading: false,
  error: null,
};

function documentReducer(state: DocumentState, action: DocumentAction): DocumentState {
  switch (action.type) {
    case 'SET_DOCUMENTS':
      return {
        ...state,
        documents: action.payload.documents,
        totalCount: action.payload.totalCount,
        isLoading: false,
        error: null,
        selectedIndex: 0,
      };

    case 'SET_PAGE':
      return { ...state, currentPage: action.payload };

    case 'SET_FILTER':
      return { ...state, filter: action.payload, currentPage: 1 };

    case 'SELECT_INDEX': {
      const maxIndex = state.documents.length - 1;
      const newIndex = Math.max(0, Math.min(action.payload, maxIndex));
      return { ...state, selectedIndex: newIndex };
    }

    case 'TOGGLE_EXPAND': {
      const newExpanded = new Set(state.expandedIds);
      if (newExpanded.has(action.payload)) {
        newExpanded.delete(action.payload);
      } else {
        newExpanded.add(action.payload);
      }
      return { ...state, expandedIds: newExpanded };
    }

    // Tree navigation actions
    case 'ENTER_TREE_NAVIGATION':
      return {
        ...state,
        treeNavigationActive: true,
        editingId: action.payload.docId,
        selectedPath: 'root',
        treeExpandedPaths: new Set(['root']),
        pendingDocument: null,
        hasUnsavedChanges: false,
      };

    case 'EXIT_TREE_NAVIGATION':
      return {
        ...state,
        treeNavigationActive: false,
        editingId: null,
        selectedPath: null,
        editingPath: null,
        editingValue: '',
      };

    case 'SELECT_PATH':
      return { ...state, selectedPath: action.payload };

    case 'TOGGLE_TREE_PATH': {
      const newTreeExpanded = new Set(state.treeExpandedPaths);
      if (newTreeExpanded.has(action.payload)) {
        newTreeExpanded.delete(action.payload);
      } else {
        newTreeExpanded.add(action.payload);
      }
      return { ...state, treeExpandedPaths: newTreeExpanded };
    }

    // Field editing actions
    case 'START_FIELD_EDIT':
      return {
        ...state,
        editingPath: action.payload.path,
        editingValue: action.payload.value,
      };

    case 'UPDATE_FIELD_VALUE':
      return { ...state, editingValue: action.payload };

    case 'APPLY_FIELD_EDIT': {
      // Get the document being edited
      const doc = state.pendingDocument ?? state.documents[state.selectedIndex];
      if (!doc) return { ...state, editingPath: null, editingValue: '' };

      // Apply the change to create a new pending document
      const parts = action.payload.path.split('.').slice(1); // Remove 'root'
      const newDoc = JSON.parse(JSON.stringify(doc)) as Document;
      let current: Record<string, unknown> = newDoc as Record<string, unknown>;

      for (let i = 0; i < parts.length - 1; i++) {
        current = current[parts[i]!] as Record<string, unknown>;
      }
      current[parts[parts.length - 1]!] = action.payload.newValue;

      return {
        ...state,
        pendingDocument: newDoc,
        hasUnsavedChanges: true,
        editingPath: null,
        editingValue: '',
      };
    }

    case 'CANCEL_FIELD_EDIT':
      return { ...state, editingPath: null, editingValue: '' };

    case 'SET_PENDING_DOCUMENT':
      return { ...state, pendingDocument: action.payload, hasUnsavedChanges: true };

    case 'SAVE_DOCUMENT_SUCCESS': {
      // Update the document in the list
      const updatedDocs = state.documents.map((doc) =>
        doc._id.toString() === action.payload._id.toString() ? action.payload : doc
      );
      return {
        ...state,
        documents: updatedDocs,
        pendingDocument: null,
        hasUnsavedChanges: false,
        treeNavigationActive: false,
        editingId: null,
      };
    }

    case 'DISCARD_DOCUMENT_CHANGES':
      return {
        ...state,
        pendingDocument: null,
        hasUnsavedChanges: false,
        treeNavigationActive: false,
        editingId: null,
        editingPath: null,
        editingValue: '',
      };

    // Legacy edit mode (keeping for backwards compat)
    case 'ENTER_EDIT_MODE':
      return {
        ...state,
        mode: 'edit',
        editingId: action.payload.id,
        editBuffer: action.payload.content,
        originalDoc: action.payload.original,
        hasUnsavedChanges: false,
      };

    case 'EXIT_EDIT_MODE':
      return { ...state, mode: 'view' };

    case 'UPDATE_EDIT_BUFFER':
      return { ...state, editBuffer: action.payload, hasUnsavedChanges: true };

    case 'SAVE_SUCCESS':
      return {
        ...state,
        mode: 'view',
        editingId: null,
        editBuffer: '',
        originalDoc: null,
        hasUnsavedChanges: false,
        pendingDocument: null,
        treeNavigationActive: false,
      };

    case 'DISCARD_CHANGES':
      return {
        ...state,
        mode: 'view',
        editingId: null,
        editBuffer: '',
        originalDoc: null,
        hasUnsavedChanges: false,
        pendingDocument: null,
        treeNavigationActive: false,
      };

    case 'SET_LOADING':
      return { ...state, isLoading: action.payload };

    case 'SET_ERROR':
      return { ...state, error: action.payload, isLoading: false };

    case 'RESET':
      return { ...defaultState };

    default:
      return state;
  }
}

interface DocumentContextValue {
  state: DocumentState;
  dispatch: React.Dispatch<DocumentAction>;
  selectNext: () => void;
  selectPrev: () => void;
  toggleSelected: () => void;
  nextPage: () => void;
  prevPage: () => void;
}

const DocumentContext = createContext<DocumentContextValue | null>(null);

export function DocumentProvider({ children }: { children: ReactNode }) {
  const [state, dispatch] = useReducer(documentReducer, defaultState);

  const selectNext = useCallback(() => {
    dispatch({ type: 'SELECT_INDEX', payload: state.selectedIndex + 1 });
  }, [state.selectedIndex]);

  const selectPrev = useCallback(() => {
    dispatch({ type: 'SELECT_INDEX', payload: state.selectedIndex - 1 });
  }, [state.selectedIndex]);

  const toggleSelected = useCallback(() => {
    const doc = state.documents[state.selectedIndex];
    if (doc) {
      dispatch({ type: 'TOGGLE_EXPAND', payload: doc._id.toString() });
    }
  }, [state.documents, state.selectedIndex]);

  const nextPage = useCallback(() => {
    const totalPages = Math.ceil(state.totalCount / state.pageSize);
    if (state.currentPage < totalPages) {
      dispatch({ type: 'SET_PAGE', payload: state.currentPage + 1 });
    }
  }, [state.currentPage, state.totalCount, state.pageSize]);

  const prevPage = useCallback(() => {
    if (state.currentPage > 1) {
      dispatch({ type: 'SET_PAGE', payload: state.currentPage - 1 });
    }
  }, [state.currentPage]);

  return (
    <DocumentContext.Provider
      value={{
        state,
        dispatch,
        selectNext,
        selectPrev,
        toggleSelected,
        nextPage,
        prevPage,
      }}
    >
      {children}
    </DocumentContext.Provider>
  );
}

export function useDocument() {
  const context = useContext(DocumentContext);
  if (!context) {
    throw new Error('useDocument must be used within DocumentProvider');
  }
  return context;
}
