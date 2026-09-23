// Run explicitly with mongosh <local-test-uri> scripts/compare-database-seed.js.
// Creates two dedicated databases for trying database compare; it refuses to overwrite them.
// Remove them with: db.getSiblingDB("openmango_dbcompare_left").dropDatabase() (and _right).
const left = db.getSiblingDB("openmango_dbcompare_left");
const right = db.getSiblingDB("openmango_dbcompare_right");
if (left.getCollectionNames().length || right.getCollectionNames().length) {
  throw new Error("openmango_dbcompare_left or _right already has collections. Drop them explicitly first.");
}
const range = (from, to) => Array.from({ length: to - from + 1 }, (_, i) => from + i);
const both = (name, docs) => {
  left.getCollection(name).insertMany(docs);
  right.getCollection(name).insertMany(docs);
};

// Identical.
both("customers", range(1, 200).map((i) => ({ _id: i, name: `Customer ${i}` })));

// 10 different, 10 left only, 20 right only.
left.orders.insertMany(range(1, 1000).map((i) => ({ _id: i, status: "paid", total: i })));
right.orders.insertMany(
  range(11, 1020).map((i) => ({ _id: i, status: i % 100 === 0 ? "refunded" : "paid", total: i })),
);

// Minor: same values, int32 on the left and double on the right.
left.products.insertMany(range(1, 50).map((i) => ({ _id: i, price: NumberInt(i) })));
right.products.insertMany(range(1, 50).map((i) => ({ _id: i, price: Double(i) })));

// Identical documents, different indexes.
both("inventory", range(1, 100).map((i) => ({ _id: i, sku: `sku-${i}` })));
left.inventory.createIndex({ sku: 1 }, { unique: true });
right.inventory.createIndex({ sku: 1 });

// One side only.
left.audit_log.insertMany(range(1, 30).map((i) => ({ _id: i, action: "login" })));
right.coupons.insertMany(range(1, 15).map((i) => ({ _id: i, code: `SAVE${i}` })));

// Listed but not compared.
for (const side of [left, right]) {
  side.createView("orders_by_status", "orders", [{ $group: { _id: "$status", count: { $sum: 1 } } }]);
  side.createCollection("metrics", { timeseries: { timeField: "ts" } });
  side.metrics.insertMany(range(1, 100).map((i) => ({ ts: new Date(Date.UTC(2026, 0, 1, 0, i)), value: i })));
}

// Large enough to scan for a few seconds, so Skip and Cancel can be tried. 20 different.
for (let start = 1; start <= 2_000_000; start += 10_000) {
  const docs = range(start, start + 9_999).map((i) => ({ _id: i, type: "click", n: i % 7 }));
  left.events.insertMany(docs);
  right.events.insertMany(docs.map((d) => (d._id % 100_000 === 0 ? { ...d, n: -1 } : d)));
}
print("Seeded openmango_dbcompare_left and openmango_dbcompare_right.");
