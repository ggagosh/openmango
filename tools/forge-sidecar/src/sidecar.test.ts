import { expect, test } from "bun:test";
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

// Use a disposable database. This suite never chooses a saved app connection.
const binary = process.env.FORGE_SIDECAR_BINARY;
const uri = process.env.FORGE_TEST_MONGODB_URI;

async function withSidecar(
  check: (request: (method: string, params?: object, timeoutMs?: number) => Promise<any>) => Promise<void>,
) {
  const child = spawn(binary!, [], { stdio: ["pipe", "pipe", "pipe"] });
  const output = createInterface({ input: child.stdout });
  const lines = output[Symbol.asyncIterator]();
  let errors = "";
  child.stderr.on("data", (data) => { errors = (errors + data).slice(-4000); });
  let id = 0;
  try {
    await check(async (method, params = {}, timeoutMs = 10_000) => {
      const requestId = ++id;
      const events = [];
      child.stdin.write(JSON.stringify({ id: requestId, method, params }) + "\n");
      let timer: ReturnType<typeof setTimeout>;
      try {
        return await Promise.race([
          (async () => {
            while (true) {
              const line = await lines.next();
              if (line.done) throw new Error(`Sidecar exited: ${errors}`);
              const message = JSON.parse(line.value);
              if (message.event) {
                events.push(message);
              } else if (message.id === requestId) {
                return { ...message, events };
              }
            }
          })(),
          new Promise((_, reject) => {
            timer = setTimeout(() => reject(new Error(`${method} timed out: ${errors}`)), timeoutMs);
          }),
        ]);
      } finally {
        clearTimeout(timer!);
      }
    });
  } finally {
    output.close();
    child.kill();
    await new Promise<void>((resolve) => child.once("close", () => resolve()));
  }
}

test.skipIf(!binary)("compiled sidecar answers RPC and reports errors", async () => {
  await withSidecar(async (request) => {
    expect(await request("ping")).toMatchObject({ ok: true, result: "pong" });
    expect(await request("unknown_method")).toMatchObject({ ok: false });
    expect(await request("evaluate", { session_id: "missing", code: "1" }))
      .toMatchObject({ ok: false, error: "Session not found: missing" });
  });
});

test.skipIf(!binary || !uri)("compiled shell preserves BSON, completion, output and sessions beyond 30 seconds", async () => {
  await withSidecar(async (request) => {
    for (const session_id of ["idle", "active"]) {
      expect(await request("create_session", { session_id, uri, database: "forge_sidecar_test" }))
        .toMatchObject({ ok: true });
      expect(await request("evaluate", { session_id, code: "const kept = 42; kept" }))
        .toMatchObject({ ok: true, result: { printable: 42 } });
    }

    const complete = await request("complete", { session_id: "active", code: "db.getCol" });
    expect(complete.ok).toBe(true);
    expect(complete.result).toContainEqual({ completion: "db.getCollection" });

    const printed = await request("evaluate", {
      session_id: "active", run_id: 7,
      code: 'print("hello"); printjson({ id: ObjectId("507f1f77bcf86cd799439011"), when: ISODate("2020-01-01") });',
    });
    expect(printed).toMatchObject({ ok: true, result: { is_undefined: true } });
    expect(printed.events).toHaveLength(2);
    expect(printed.events[0]).toMatchObject({ run_id: 7, lines: ["hello"] });
    expect(printed.events[1].payload).toEqual([{
      id: { $oid: "507f1f77bcf86cd799439011" },
      when: { $date: "2020-01-01T00:00:00Z" },
    }]);
    expect(await request("evaluate", { session_id: "active", code: "null" }))
      .toMatchObject({ ok: true, result: { printable: null, is_undefined: false } });

    // The old global idle timer closed both the running query and the idle shell.
    expect(await request("evaluate", {
      session_id: "active", code: "(async () => { await sleep(31_000); return kept + (await db.runCommand({ ping: 1 })).ok; })()",
    }, 40_000)).toMatchObject({ ok: true, result: { printable: 43 } });
    expect(await request("evaluate", { session_id: "idle", code: "kept" }))
      .toMatchObject({ ok: true, result: { printable: 42 } });

    for (const session_id of ["idle", "active"]) {
      expect(await request("dispose_session", { session_id })).toMatchObject({ ok: true });
      expect(await request("evaluate", { session_id, code: "kept" }))
        .toMatchObject({ ok: false, error: `Session not found: ${session_id}` });
    }
  });
}, 60_000);
