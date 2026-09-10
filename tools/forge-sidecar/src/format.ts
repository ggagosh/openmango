import { EJSON } from "bson";

export function safePrintable(value: unknown) {
  if (value === undefined) return null;
  try {
    return EJSON.serialize(value, { relaxed: true });
  } catch {
    // Fall back for values that cannot be represented as BSON.
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

export function formatPrintValue(value: unknown, kind: "print" | "printjson"): string {
  const printable = value && typeof value === "object" && "printable" in value
    ? value.printable
    : value;
  if (printable === undefined) return "undefined";
  try {
    if (kind === "printjson") {
      return EJSON.stringify(printable, { relaxed: true, indent: 2 });
    }
    if (typeof printable === "string") return printable;
    return EJSON.stringify(printable, { relaxed: true });
  } catch {
    try {
      return JSON.stringify(printable, null, kind === "printjson" ? 2 : undefined);
    } catch {
      return String(printable);
    }
  }
}
