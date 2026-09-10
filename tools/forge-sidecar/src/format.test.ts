import { expect, test } from "bun:test";
import { ObjectId } from "bson";
import { formatPrintValue } from "./format";

test("printed undefined and null remain distinct", () => {
  expect(formatPrintValue({ printable: undefined }, "print")).toBe("undefined");
  expect(formatPrintValue({ printable: null }, "print")).toBe("null");
});

test("print preserves text while printjson keeps JSON quoting", () => {
  const value = { printable: "hello\nworld" };
  expect(formatPrintValue(value, "print")).toBe("hello\nworld");
  expect(formatPrintValue(value, "printjson")).toBe('"hello\\nworld"');
});

test("printed documents preserve BSON and fields named printable", () => {
  const id = new ObjectId("507f1f77bcf86cd799439011");
  const text = formatPrintValue({ printable: { _id: id, printable: "kept" } }, "printjson");
  expect(JSON.parse(text)).toEqual({ _id: { $oid: id.toHexString() }, printable: "kept" });
});
