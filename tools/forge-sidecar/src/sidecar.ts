import { ElectronRuntime } from "@mongosh/browser-runtime-electron";
import { CompassServiceProvider } from "@mongosh/service-provider-node-driver";
import { EJSON } from "bson";
import { EventEmitter } from "events";
import readline from "readline";

type Session = {
  runtime: ElectronRuntime;
  provider: CompassServiceProvider;
  uri: string;
  database: string;
  currentRunId?: number | null;
};

type RequestMessage = {
  id: number;
  method: string;
  params?: Record<string, unknown>;
};

type ResponseMessage = {
  id: number;
  ok: boolean;
  result?: unknown;
  error?: string;
};

const sessions = new Map<string, Session>();

function send(message: ResponseMessage) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function sendEvent(message: Record<string, unknown>) {
  process.stdout.write(`${JSON.stringify(message)}\n`);
}

function safePrintable(value: unknown) {
  if (value === undefined) {
    return null;
  }
  try {
    return EJSON.serialize(value as any, { relaxed: true });
  } catch {
    // fall through
  }
  try {
    return JSON.parse(JSON.stringify(value));
  } catch {
    try {
      return String(value);
    } catch {
      return null;
    }
  }
}

function requireSession(sessionId: string): Session {
  const session = sessions.get(sessionId);
  if (!session) {
    throw new Error(`Session not found: ${sessionId}`);
  }
  return session;
}

function formatPrintValue(value: unknown, kind: "print" | "printjson"): string {
  const printable =
    value && typeof value === "object" && "printable" in (value as any)
      ? (value as any).printable
      : value;
  try {
    if (kind === "printjson") {
      return EJSON.stringify(printable as any, { relaxed: true, indent: 2 });
    }
    if (typeof printable === "string") {
      return printable;
    }
    return EJSON.stringify(printable as any, { relaxed: true });
  } catch {
    try {
      return JSON.stringify(printable, null, kind === "printjson" ? 2 : undefined);
    } catch {
      return String(printable);
    }
  }
}

function splitLines(text: string): string[] {
  return text.split(/\r?\n/);
}

async function createSession(params: Record<string, unknown>) {
  const sessionId = params.session_id as string | undefined;
  const uri = params.uri as string | undefined;
  const database = (params.database as string | undefined) ?? "";
  const driverOptions = (params.driver_options as Record<string, unknown> | undefined) ?? {};

  if (!sessionId) {
    throw new Error("create_session missing session_id");
  }
  if (!uri) {
    throw new Error("create_session missing uri");
  }

  if (sessions.has(sessionId)) {
    await disposeSession({ session_id: sessionId });
  }

  const bus = new EventEmitter();
  const provider = await CompassServiceProvider.connect(
    uri,
    {
      productName: "OpenMango",
      productDocsLink: "https://github.com/ggagosh/openmango",
      appName: "OpenMango",
      ...driverOptions,
    },
    {},
    bus
  );

  const runtime = new ElectronRuntime(provider, bus);

  runtime.setEvaluationListener({
    onPrint(values: any[], type?: string) {
      const kind =
        typeof type === "string" && type.toLowerCase().includes("json")
          ? "printjson"
          : "print";
      const rendered = values
        .map((value) => formatPrintValue(value, kind))
        .join(kind === "print" ? " " : "\n");
      const lines = splitLines(rendered);
      const runId = sessions.get(sessionId)?.currentRunId ?? null;
      const payload =
        kind === "printjson"
          ? values.map((value) =>
              safePrintable(
                value && typeof value === "object" && "printable" in value
                  ? (value as any).printable
                  : value
              )
            )
          : undefined;
      sendEvent({
        event: "print",
        session_id: sessionId,
        run_id: runId,
        kind,
        lines,
        payload,
      });
    },
    onClearCommand() {
      sendEvent({
        event: "clear",
        session_id: sessionId,
      });
    },
  });

  if (database) {
    await runtime.evaluate(`db = db.getSiblingDB(${JSON.stringify(database)})`);
  }

  sessions.set(sessionId, {
    runtime,
    provider,
    uri,
    database,
    currentRunId: null,
  });
  return true;
}

async function disposeSession(params: Record<string, unknown>) {
  const sessionId = params.session_id as string | undefined;
  if (!sessionId) {
    throw new Error("dispose_session missing session_id");
  }

  const session = sessions.get(sessionId);
  if (!session) {
    return false;
  }

  await session.provider.close();
  sessions.delete(sessionId);
  return true;
}

async function resetSession(params: Record<string, unknown>) {
  const sessionId = params.session_id as string | undefined;
  if (!sessionId) {
    throw new Error("reset_session missing session_id");
  }

  const session = sessions.get(sessionId);
  if (!session) {
    throw new Error(`Session not found: ${sessionId}`);
  }

  await disposeSession({ session_id: sessionId });
  return await createSession({
    session_id: sessionId,
    uri: session.uri,
    database: session.database,
  });
}

async function evaluate(params: Record<string, unknown>) {
  const sessionId = params.session_id as string | undefined;
  const code = params.code as string | undefined;
  const runId = typeof params.run_id === "number" ? params.run_id : null;
  if (!sessionId || code === undefined) {
    throw new Error("evaluate missing session_id or code");
  }

  const session = requireSession(sessionId);
  session.currentRunId = runId;
  try {
    const result = await session.runtime.evaluate(code);
    return {
      ...result,
      run_id: runId,
      printable: safePrintable(result.printable),
    };
  } finally {
    session.currentRunId = null;
  }
}

async function complete(params: Record<string, unknown>) {
  const sessionId = params.session_id as string | undefined;
  const code = params.code as string | undefined;
  if (!sessionId || code === undefined) {
    throw new Error("complete missing session_id or code");
  }

  const session = requireSession(sessionId);
  return await session.runtime.getCompletions(code);
}

async function handleRequest(req: RequestMessage) {
  switch (req.method) {
    case "create_session":
      return await createSession(req.params ?? {});
    case "dispose_session":
      return await disposeSession(req.params ?? {});
    case "reset_session":
      return await resetSession(req.params ?? {});
    case "evaluate":
      return await evaluate(req.params ?? {});
    case "complete":
      return await complete(req.params ?? {});
    case "ping":
      return "pong";
    default:
      throw new Error(`Unknown method: ${req.method}`);
  }
}

const rl = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });

rl.on("line", async (line) => {
  const trimmed = line.trim();
  if (!trimmed) return;

  let request: RequestMessage;
  try {
    request = JSON.parse(trimmed);
  } catch (err) {
    console.error("[forge-sidecar] Invalid JSON:", err);
    return;
  }

  if (typeof request.id !== "number" || !request.method) {
    console.error("[forge-sidecar] Invalid request:", request);
    return;
  }

  try {
    const result = await handleRequest(request);
    send({ id: request.id, ok: true, result });
  } catch (err) {
    send({
      id: request.id,
      ok: false,
      error: err instanceof Error ? err.message : String(err),
    });
  }
});

rl.on("close", () => {
  process.exit(0);
});
