// Seeds the collections the memory benchmark opens. Run with:
//   mongosh --quiet mongodb://localhost:27018 scripts/bench-seed.js

const bench = db.getSiblingDB("bench");
bench.dropDatabase();

const STATUS = ["pending", "paid", "shipped", "delivered", "cancelled"];
for (let start = 0; start < 1_000_000; start += 10_000) {
  const batch = [];
  for (let i = start; i < start + 10_000; i++) {
    batch.push({
      user_id: i % 50_000,
      email: `user${i % 50_000}@example.com`,
      status: STATUS[i % 5],
      amount: (i % 100_000) / 100,
      created_at: new Date(1_700_000_000_000 + i * 1000),
      tags: ["web", "promo", "returning"],
      address: { city: "Tbilisi", zip: String(i % 10_000).padStart(4, "0"), country: "GE" },
    });
  }
  bench.orders.insertMany(batch, { ordered: false });
}

const item = (i) => ({
  sku: `SKU-${String(i).padStart(6, "0")}`,
  qty: i % 9,
  price: i / 7,
  note: "x".repeat(200),
  attrs: { color: "green", size: "M" },
});

// One document of about 13 MB.
bench.fat.insertOne({ kind: "fat", items: Array.from({ length: 45_000 }, (_, i) => item(i)) });

// 50 documents of about 1 MB each, so one page is about 50 MB.
bench.big.insertMany(
  Array.from({ length: 50 }, (_, n) => ({ n, rows: Array.from({ length: 3_400 }, (_, i) => item(i)) })),
);

// Log every operation, so a run can be checked against the server log.
bench.setProfilingLevel(0, { slowms: 0 });

for (const name of ["orders", "fat", "big"]) {
  const stats = bench.getCollection(name).stats();
  print(`${name}: ${stats.count} documents, ${(stats.size / 1048576).toFixed(1)} MB`);
}
