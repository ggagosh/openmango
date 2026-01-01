// Connection types
import type { ParsedConnectionString } from './connection-string.ts';

export type ConnectionStatus = 'disconnected' | 'connecting' | 'connected' | 'error';

export interface Connection {
  id: string;
  alias: string;
  url: string;
  status: ConnectionStatus;
  error?: string;
  parsed?: ParsedConnectionString;
  createdAt: string;
  lastConnectedAt?: string;
}

// Re-export connection string types
export type {
  MongoScheme,
  MongoHost,
  MongoConnectionOptions,
  ParsedConnectionString,
  ValidationResult,
  TestConnectionResult,
} from './connection-string.ts';

// Navigation tree types
export type NodeType = 'connection' | 'database' | 'collection';

export interface TreeNodeData {
  id: string;
  type: NodeType;
  name: string;
  parentId: string | null;
  isExpanded: boolean;
  children: TreeNodeData[];
  connectionId?: string;
  databaseName?: string;
  collectionName?: string;
  status?: ConnectionStatus; // For connection nodes
}

// Focus management
export type FocusZone = 'sidebar' | 'panel' | 'modal';

// Modal types
export type ModalType =
  | 'add-connection'
  | 'edit-connection'
  | 'create-database'
  | 'create-collection'
  | 'copy-database'
  | 'copy-collection'
  | 'confirm';

export interface ModalState {
  type: ModalType;
  props?: Record<string, unknown>;
}

// Copy operation options
export interface CopyDatabaseOptions {
  targetConnectionId: string;
  dropExisting: boolean;
  includeIndexes: boolean;
  selectedCollections: string[];
}

export interface CopyCollectionOptions {
  targetConnectionId: string;
  targetDatabase: string;
  dropExisting: boolean;
  includeIndexes: boolean;
  newName?: string;
  filterQuery?: string;
  limit?: number;
}

// Stats types for detail panels
export interface DatabaseStats {
  sizeOnDisk: string;
  collectionCount: number;
  documentCount: number;
}

export interface CollectionStats {
  documentCount: number;
  avgDocumentSize: string;
  totalSize: string;
  indexCount: number;
}

// Re-export document types
export type {
  EditorMode,
  MongoDocument,
  DocumentBrowserState,
  FindDocumentsOptions,
  FindDocumentsResult,
  JsonToken,
  HighlightedSegment,
} from './document.ts';
