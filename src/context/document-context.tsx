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
  editingId: string | null;
  editBuffer: string;
  originalDoc: Document | null;
  hasUnsavedChanges: boolean;
  isLoading: boolean;
  error: string | null;
}

type DocumentAction =
  | { type: 'SET_DOCUMENTS'; payload: { documents: WithId<Document>[]; totalCount: number } }
  | { type: 'SET_PAGE'; payload: number }
  | { type: 'SET_FILTER'; payload: string }
  | { type: 'SELECT_INDEX'; payload: number }
  | { type: 'TOGGLE_EXPAND'; payload: string }
  | { type: 'ENTER_EDIT_MODE'; payload: { id: string; content: string; original: Document } }
  | { type: 'EXIT_EDIT_MODE' }
  | { type: 'UPDATE_EDIT_BUFFER'; payload: string }
  | { type: 'SAVE_SUCCESS' }
  | { type: 'DISCARD_CHANGES' }
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
  editingId: null,
  editBuffer: '',
  originalDoc: null,
  hasUnsavedChanges: false,
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
      return {
        ...state,
        mode: 'view',
      };

    case 'UPDATE_EDIT_BUFFER':
      return {
        ...state,
        editBuffer: action.payload,
        hasUnsavedChanges: true,
      };

    case 'SAVE_SUCCESS':
      return {
        ...state,
        mode: 'view',
        editingId: null,
        editBuffer: '',
        originalDoc: null,
        hasUnsavedChanges: false,
      };

    case 'DISCARD_CHANGES':
      return {
        ...state,
        mode: 'view',
        editingId: null,
        editBuffer: '',
        originalDoc: null,
        hasUnsavedChanges: false,
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
