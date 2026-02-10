// Bun compile entry — patch unimplemented Node stubs before anything loads.
//
// Bun <1.4 throws NotImplementedError on v8.startupSnapshot.isBuildingSnapshot
// which mongosh's runtime-support calls outside of a try/catch.
// Patching MUST happen before sidecar.ts (and its transitive deps) evaluate,
// so we use dynamic import() to avoid ES module hoisting.
try {
  const v8: any = require("v8");
  if (v8?.startupSnapshot) {
    Object.defineProperty(v8.startupSnapshot, "isBuildingSnapshot", {
      value: () => false,
      writable: true,
      configurable: true,
    });
  }
} catch {
  // v8 module not available — fine
}

await import("./sidecar");
