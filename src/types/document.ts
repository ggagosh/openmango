import type { Document, ObjectId, WithId } from 'mongodb';

export type EditorMode = 'view' | 'edit';

export interface MongoDocument extends Document {
  _id: ObjectId;
}

export interface DocumentBrowserState {
  mode: EditorMode;
  documents: WithId<Document>[];
  totalCount: number;
  currentPage: number;
  pageSize: number;
  filter: string;
  selectedIndex: number;
  expandedIds: Set<string>;
  pendingChanges: Map<string, Document>;
  isLoading: boolean;
  error: string | null;
}

export interface FindDocumentsOptions {
  filter?: string;
  projection?: Record<string, 0 | 1>;
  sort?: Record<string, 1 | -1>;
  limit?: number;
  skip?: number;
}

export interface FindDocumentsResult {
  documents: WithId<Document>[];
  totalCount: number;
}

export interface JsonToken {
  type:
    | 'key'
    | 'string'
    | 'number'
    | 'boolean'
    | 'null'
    | 'bracket'
    | 'colon'
    | 'comma'
    | 'objectId'
    | 'date';
  value: string;
}

export interface HighlightedSegment {
  text: string;
  color: string;
}
