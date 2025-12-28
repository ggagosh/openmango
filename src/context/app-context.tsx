import { type ReactNode, createContext, useContext, useReducer } from 'react';
import type { Connection, TreeNodeData } from '../types/index.ts';

interface AppState {
  connections: Connection[];
  navigationTree: TreeNodeData[];
  selectedNodeId: string | null;
  statusMessage: string | null;
  statusType: 'info' | 'error' | 'success';
  isLoading: boolean;
  searchQuery: string;
  isSearching: boolean;
}

type AppAction =
  | { type: 'ADD_CONNECTION'; payload: Connection }
  | { type: 'UPDATE_CONNECTION'; payload: Partial<Connection> & { id: string } }
  | { type: 'REMOVE_CONNECTION'; payload: string }
  | { type: 'SET_TREE'; payload: TreeNodeData[] }
  | { type: 'SELECT_NODE'; payload: string | null }
  | { type: 'TOGGLE_EXPAND'; payload: string }
  | { type: 'SET_STATUS'; payload: { message: string; type: 'info' | 'error' | 'success' } }
  | { type: 'CLEAR_STATUS' }
  | { type: 'SET_LOADING'; payload: boolean }
  | { type: 'SET_SEARCH'; payload: string }
  | { type: 'START_SEARCH' }
  | { type: 'END_SEARCH' }
  | { type: 'CONNECT_START'; payload: string }
  | { type: 'CONNECT_SUCCESS'; payload: { id: string; databases: string[] } }
  | { type: 'CONNECT_ERROR'; payload: { id: string; error: string } }
  | { type: 'DISCONNECT'; payload: string }
  | { type: 'UPDATE_NODE_CHILDREN'; payload: { nodeId: string; children: TreeNodeData[] } }
  | { type: 'ADD_CONNECTION_WITH_NODE'; payload: { connection: Connection } };

const defaultState: AppState = {
  connections: [],
  isLoading: false,
  isSearching: false,
  navigationTree: [],
  searchQuery: '',
  selectedNodeId: null,
  statusMessage: null,
  statusType: 'info',
};

function toggleNodeExpand(nodes: TreeNodeData[], nodeId: string): TreeNodeData[] {
  return nodes.map((node) => {
    if (node.id === nodeId) {
      return { ...node, isExpanded: !node.isExpanded };
    }
    if (node.children.length > 0) {
      return { ...node, children: toggleNodeExpand(node.children, nodeId) };
    }
    return node;
  });
}

function updateNodeChildren(
  nodes: TreeNodeData[],
  nodeId: string,
  children: TreeNodeData[]
): TreeNodeData[] {
  return nodes.map((node) => {
    if (node.id === nodeId) {
      return { ...node, children };
    }
    if (node.children.length > 0) {
      return { ...node, children: updateNodeChildren(node.children, nodeId, children) };
    }
    return node;
  });
}

function appReducer(state: AppState, action: AppAction): AppState {
  switch (action.type) {
    case 'ADD_CONNECTION':
      return { ...state, connections: [...state.connections, action.payload] };

    case 'UPDATE_CONNECTION':
      return {
        ...state,
        connections: state.connections.map((conn) =>
          conn.id === action.payload.id ? { ...conn, ...action.payload } : conn
        ),
      };

    case 'REMOVE_CONNECTION':
      return {
        ...state,
        connections: state.connections.filter((conn) => conn.id !== action.payload),
        navigationTree: state.navigationTree.filter((node) => node.connectionId !== action.payload),
      };

    case 'SET_TREE':
      return { ...state, navigationTree: action.payload };

    case 'SELECT_NODE':
      return { ...state, selectedNodeId: action.payload };

    case 'TOGGLE_EXPAND':
      return {
        ...state,
        navigationTree: toggleNodeExpand(state.navigationTree, action.payload),
      };

    case 'SET_STATUS':
      return {
        ...state,
        statusMessage: action.payload.message,
        statusType: action.payload.type,
      };

    case 'CLEAR_STATUS':
      return { ...state, statusMessage: null };

    case 'SET_LOADING':
      return { ...state, isLoading: action.payload };

    case 'SET_SEARCH':
      return { ...state, searchQuery: action.payload };

    case 'START_SEARCH':
      return { ...state, isSearching: true };

    case 'END_SEARCH':
      return { ...state, isSearching: false };

    case 'CONNECT_START':
      return {
        ...state,
        isLoading: true,
        connections: state.connections.map((conn) =>
          conn.id === action.payload ? { ...conn, status: 'connecting' as const } : conn
        ),
        navigationTree: state.navigationTree.map((node) =>
          node.id === action.payload ? { ...node, status: 'connecting' as const } : node
        ),
      };

    case 'CONNECT_SUCCESS': {
      const connection = state.connections.find((c) => c.id === action.payload.id);
      const databaseNodes: TreeNodeData[] = action.payload.databases.map((dbName) => ({
        id: `${action.payload.id}:${dbName}`,
        type: 'database' as const,
        name: dbName,
        parentId: action.payload.id,
        isExpanded: false,
        children: [],
        connectionId: action.payload.id,
        databaseName: dbName,
      }));

      return {
        ...state,
        isLoading: false,
        connections: state.connections.map((conn) =>
          conn.id === action.payload.id
            ? { ...conn, status: 'connected' as const, error: undefined }
            : conn
        ),
        navigationTree: state.navigationTree.map((node) =>
          node.id === action.payload.id
            ? { ...node, status: 'connected' as const, children: databaseNodes, isExpanded: true }
            : node
        ),
        statusMessage: `Connected to ${connection?.alias ?? 'database'}`,
        statusType: 'success',
      };
    }

    case 'CONNECT_ERROR':
      return {
        ...state,
        isLoading: false,
        connections: state.connections.map((conn) =>
          conn.id === action.payload.id
            ? { ...conn, status: 'error' as const, error: action.payload.error }
            : conn
        ),
        navigationTree: state.navigationTree.map((node) =>
          node.id === action.payload.id ? { ...node, status: 'error' as const } : node
        ),
        statusMessage: action.payload.error,
        statusType: 'error',
      };

    case 'DISCONNECT':
      return {
        ...state,
        connections: state.connections.map((conn) =>
          conn.id === action.payload ? { ...conn, status: 'disconnected' as const } : conn
        ),
        navigationTree: state.navigationTree.map((node) =>
          node.id === action.payload
            ? { ...node, status: 'disconnected' as const, children: [], isExpanded: false }
            : node
        ),
      };

    case 'UPDATE_NODE_CHILDREN':
      return {
        ...state,
        navigationTree: updateNodeChildren(
          state.navigationTree,
          action.payload.nodeId,
          action.payload.children
        ),
      };

    case 'ADD_CONNECTION_WITH_NODE': {
      const { connection } = action.payload;
      const treeNode: TreeNodeData = {
        id: connection.id,
        type: 'connection',
        name: connection.alias,
        parentId: null,
        isExpanded: false,
        children: [],
        connectionId: connection.id,
        status: 'disconnected',
      };
      return {
        ...state,
        connections: [...state.connections, connection],
        navigationTree: [...state.navigationTree, treeNode],
      };
    }

    default:
      return state;
  }
}

interface AppContextValue {
  state: AppState;
  dispatch: React.Dispatch<AppAction>;
}

const AppContext = createContext<AppContextValue | null>(null);

interface AppProviderProps {
  children: ReactNode;
  initialConnections?: Connection[];
  initialTree?: TreeNodeData[];
}

export function AppProvider({ children, initialConnections, initialTree }: AppProviderProps) {
  const initialState: AppState = {
    ...defaultState,
    connections: initialConnections ?? [],
    navigationTree: initialTree ?? [],
  };

  const [state, dispatch] = useReducer(appReducer, initialState);

  return <AppContext.Provider value={{ dispatch, state }}>{children}</AppContext.Provider>;
}

export function useApp() {
  const context = useContext(AppContext);
  if (!context) {
    throw new Error('useApp must be used within AppProvider');
  }
  return context;
}
