// Run explicitly with mongosh <local-test-uri> scripts/compare-bench-seed.js.
// This only creates a dedicated benchmark database; it refuses to overwrite existing data.
const benchmark = db.getSiblingDB("openmango_compare_bench");
const names = ["left", "right", "right_sku"];
if (names.some((name) => benchmark.getCollectionNames().includes(name))) {
  throw new Error("Benchmark collections already exist. Remove them explicitly before reseeding.");
}
const payload = "x".repeat(900);
for (let start = 0; start < 1_000_000; start += 1_000) {
  const left = [];
  const right = [];
  const rightSku = [];
  for (let id = start; id < start + 1_000; id++) {
    const document = { _id: id, sku: `sku-${String(id).padStart(7, "0")}`, unindexed: id, payload, value: 0 };
    left.push(document);
    right.push({ ...document, value: id % 100 === 0 ? 1 : 0 });
    rightSku.push({ ...document, _id: id + 1_000_000, value: id % 100 === 0 ? 1 : 0 });
  }
  benchmark.left.insertMany(left);
  benchmark.right.insertMany(right);
  benchmark.right_sku.insertMany(rightSku);
  if (start % 100_000 === 0) print(`${start + 1_000} documents per collection`);
}
for (const name of names) benchmark.getCollection(name).createIndex({ sku: 1 }, { unique: true });
print("Seeded openmango_compare_bench: 1M documents per collection, 1% changed.");
