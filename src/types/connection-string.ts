// MongoDB Connection String Types
// Based on: https://specifications.readthedocs.io/en/latest/connection-string/connection-string-spec/

export type MongoScheme = 'mongodb' | 'mongodb+srv';

export interface MongoHost {
  hostname: string;
  port?: number; // Default 27017 for mongodb://, DNS resolved for +srv
}

export interface MongoConnectionOptions {
  // SSL/TLS
  tls?: boolean;
  tlsAllowInvalidCertificates?: boolean;
  tlsCAFile?: string;

  // Authentication
  authSource?: string;
  authMechanism?: 'SCRAM-SHA-1' | 'SCRAM-SHA-256' | 'MONGODB-X509' | 'GSSAPI' | 'PLAIN';

  // Replica Set
  replicaSet?: string;

  // Timeouts (in ms)
  connectTimeoutMS?: number;
  serverSelectionTimeoutMS?: number;
  socketTimeoutMS?: number;

  // Connection Pool
  maxPoolSize?: number;
  minPoolSize?: number;

  // Read/Write Preferences
  readPreference?: 'primary' | 'primaryPreferred' | 'secondary' | 'secondaryPreferred' | 'nearest';
  w?: string | number;

  // Other common options
  retryWrites?: boolean;
  directConnection?: boolean;

  // Raw options map for any unrecognized options
  raw: Record<string, string>;
}

export interface ParsedConnectionString {
  scheme: MongoScheme;
  hosts: MongoHost[];
  username?: string; // Never store password
  authDatabase?: string;
  options: MongoConnectionOptions;

  // Derived display values
  displayHost: string; // First host or "cluster" for multi-host
  isReplicaSet: boolean;
  isSRV: boolean;
}

export interface ValidationResult {
  isValid: boolean;
  error?: string;
  errorType?: 'format' | 'encoding' | 'auth_database' | 'host' | 'scheme';
  parsed?: ParsedConnectionString;
}

export interface TestConnectionResult {
  success: boolean;
  error?: string;
  errorType?: 'network' | 'auth' | 'timeout' | 'dns' | 'ssl' | 'unknown';
  latencyMs?: number;
  serverInfo?: {
    version?: string;
    isReplicaSet?: boolean;
    replicaSetName?: string;
  };
}
