import type {
  MongoScheme,
  MongoHost,
  MongoConnectionOptions,
  ParsedConnectionString,
  ValidationResult,
} from '../types/connection-string.ts';

// Invalid characters for auth database per MongoDB spec
const INVALID_AUTH_DB_CHARS = /[/\\ "$]/;

/**
 * Parse and validate a MongoDB connection string
 * Based on: https://specifications.readthedocs.io/en/latest/connection-string/connection-string-spec/
 */
export function parseConnectionString(url: string): ValidationResult {
  const trimmed = url.trim();

  // Check scheme
  let scheme: MongoScheme;
  let remainder: string;

  if (trimmed.startsWith('mongodb+srv://')) {
    scheme = 'mongodb+srv';
    remainder = trimmed.slice('mongodb+srv://'.length);
  } else if (trimmed.startsWith('mongodb://')) {
    scheme = 'mongodb';
    remainder = trimmed.slice('mongodb://'.length);
  } else {
    return {
      isValid: false,
      error: 'URL must start with mongodb:// or mongodb+srv://',
      errorType: 'scheme',
    };
  }

  if (!remainder) {
    return {
      isValid: false,
      error: 'Missing host information',
      errorType: 'host',
    };
  }

  // Split userinfo from rest (find the last @ before any / or ?)
  let userInfo: string | undefined;
  let hostAndRest: string;

  const slashIndex = remainder.indexOf('/');
  const questionIndex = remainder.indexOf('?');
  const pathStart =
    slashIndex !== -1 ? slashIndex : questionIndex !== -1 ? questionIndex : remainder.length;
  const hostSection = remainder.slice(0, pathStart);
  const atIndex = hostSection.lastIndexOf('@');

  if (atIndex !== -1) {
    userInfo = hostSection.slice(0, atIndex);
    hostAndRest = hostSection.slice(atIndex + 1) + remainder.slice(pathStart);
  } else {
    hostAndRest = remainder;
  }

  // Parse username (never store password)
  let username: string | undefined;
  if (userInfo) {
    const colonIndex = userInfo.indexOf(':');
    const rawUsername = colonIndex !== -1 ? userInfo.slice(0, colonIndex) : userInfo;

    try {
      username = decodeURIComponent(rawUsername);
    } catch {
      return {
        isValid: false,
        error: 'Invalid percent-encoding in username',
        errorType: 'encoding',
      };
    }
  }

  // Split host(s) from path/query
  const pathSlashIndex = hostAndRest.indexOf('/');
  const pathQuestionIndex = hostAndRest.indexOf('?');

  let hostsPart: string;
  let pathPart: string | undefined;
  let queryPart: string | undefined;

  if (pathSlashIndex !== -1) {
    hostsPart = hostAndRest.slice(0, pathSlashIndex);
    const afterSlash = hostAndRest.slice(pathSlashIndex + 1);
    const qIndex = afterSlash.indexOf('?');
    if (qIndex !== -1) {
      pathPart = afterSlash.slice(0, qIndex);
      queryPart = afterSlash.slice(qIndex + 1);
    } else {
      pathPart = afterSlash;
    }
  } else if (pathQuestionIndex !== -1) {
    hostsPart = hostAndRest.slice(0, pathQuestionIndex);
    queryPart = hostAndRest.slice(pathQuestionIndex + 1);
  } else {
    hostsPart = hostAndRest;
  }

  // Parse hosts
  const hosts: MongoHost[] = [];
  const hostStrings = hostsPart.split(',');

  for (const hostStr of hostStrings) {
    const trimmedHost = hostStr.trim();
    if (!trimmedHost) {
      return {
        isValid: false,
        error: 'Empty host in connection string',
        errorType: 'host',
      };
    }

    // Handle IPv6 addresses [::1]:port
    let hostname: string;
    let port: number | undefined;

    if (trimmedHost.startsWith('[')) {
      const closeBracket = trimmedHost.indexOf(']');
      if (closeBracket === -1) {
        return {
          isValid: false,
          error: 'Invalid IPv6 address format',
          errorType: 'host',
        };
      }
      hostname = trimmedHost.slice(1, closeBracket);
      const afterBracket = trimmedHost.slice(closeBracket + 1);
      if (afterBracket.startsWith(':')) {
        port = parseInt(afterBracket.slice(1), 10);
        if (isNaN(port) || port < 1 || port > 65535) {
          return {
            isValid: false,
            error: `Invalid port number: ${afterBracket.slice(1)}`,
            errorType: 'host',
          };
        }
      }
    } else {
      const colonIndex = trimmedHost.lastIndexOf(':');
      if (colonIndex !== -1) {
        hostname = trimmedHost.slice(0, colonIndex);
        const portStr = trimmedHost.slice(colonIndex + 1);
        port = parseInt(portStr, 10);
        if (isNaN(port) || port < 1 || port > 65535) {
          return {
            isValid: false,
            error: `Invalid port number: ${portStr}`,
            errorType: 'host',
          };
        }
      } else {
        hostname = trimmedHost;
      }
    }

    if (!hostname) {
      return {
        isValid: false,
        error: 'Empty hostname',
        errorType: 'host',
      };
    }

    hosts.push({ hostname, port });
  }

  // SRV connections must have exactly one host with no port
  if (scheme === 'mongodb+srv') {
    if (hosts.length !== 1) {
      return {
        isValid: false,
        error: 'mongodb+srv:// must have exactly one host',
        errorType: 'host',
      };
    }
    if (hosts[0]!.port !== undefined) {
      return {
        isValid: false,
        error: 'mongodb+srv:// must not specify a port',
        errorType: 'host',
      };
    }
  }

  // Parse auth database
  let authDatabase: string | undefined;
  if (pathPart) {
    try {
      authDatabase = decodeURIComponent(pathPart);
    } catch {
      return {
        isValid: false,
        error: 'Invalid percent-encoding in auth database',
        errorType: 'encoding',
      };
    }

    if (authDatabase && INVALID_AUTH_DB_CHARS.test(authDatabase)) {
      return {
        isValid: false,
        error: 'Auth database cannot contain: / \\ space " $',
        errorType: 'auth_database',
      };
    }
  }

  // Parse query options
  const options: MongoConnectionOptions = { raw: {} };
  if (queryPart) {
    const params = new URLSearchParams(queryPart);
    for (const [key, value] of params) {
      switch (key) {
        case 'tls':
        case 'ssl':
          options.tls = value === 'true';
          break;
        case 'tlsAllowInvalidCertificates':
          options.tlsAllowInvalidCertificates = value === 'true';
          break;
        case 'tlsCAFile':
          options.tlsCAFile = value;
          break;
        case 'authSource':
          options.authSource = value;
          break;
        case 'authMechanism':
          options.authMechanism = value as MongoConnectionOptions['authMechanism'];
          break;
        case 'replicaSet':
          options.replicaSet = value;
          break;
        case 'connectTimeoutMS':
          options.connectTimeoutMS = parseInt(value, 10);
          break;
        case 'serverSelectionTimeoutMS':
          options.serverSelectionTimeoutMS = parseInt(value, 10);
          break;
        case 'socketTimeoutMS':
          options.socketTimeoutMS = parseInt(value, 10);
          break;
        case 'maxPoolSize':
          options.maxPoolSize = parseInt(value, 10);
          break;
        case 'minPoolSize':
          options.minPoolSize = parseInt(value, 10);
          break;
        case 'readPreference':
          options.readPreference = value as MongoConnectionOptions['readPreference'];
          break;
        case 'w':
          options.w = isNaN(parseInt(value, 10)) ? value : parseInt(value, 10);
          break;
        case 'retryWrites':
          options.retryWrites = value === 'true';
          break;
        case 'directConnection':
          options.directConnection = value === 'true';
          break;
        default:
          options.raw[key] = value;
      }
    }
  }

  // SRV connections have TLS enabled by default
  if (scheme === 'mongodb+srv' && options.tls === undefined) {
    options.tls = true;
  }

  const parsed: ParsedConnectionString = {
    scheme,
    hosts,
    username,
    authDatabase,
    options,
    displayHost: hosts.length === 1 ? hosts[0]!.hostname : `${hosts.length} hosts`,
    isReplicaSet: hosts.length > 1 || options.replicaSet !== undefined,
    isSRV: scheme === 'mongodb+srv',
  };

  return {
    isValid: true,
    parsed,
  };
}

/**
 * Sanitize a connection string for display (redact password)
 */
export function sanitizeConnectionString(url: string): string {
  try {
    // Match pattern: scheme://username:password@rest
    const credentialPattern = /^(mongodb(?:\+srv)?:\/\/)([^:]+):([^@]+)@(.+)$/;
    const match = url.match(credentialPattern);

    if (match) {
      const [, schemePrefix, username, , rest] = match;
      return `${schemePrefix}${username}:****@${rest}`;
    }

    return url;
  } catch {
    return url;
  }
}
