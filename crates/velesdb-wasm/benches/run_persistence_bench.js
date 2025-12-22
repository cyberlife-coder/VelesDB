/**
 * Persistence Benchmark Runner
 * 
 * Run with: node benches/run_persistence_bench.js
 */

const { VectorStore } = require('../pkg/velesdb_wasm.js');

console.log('\n🚀 VelesDB Persistence Benchmarks\n');
console.log('============================================================\n');

function measure(name, fn, iterations = 100) {
  const start = performance.now();
  for (let i = 0; i < iterations; i++) {
    fn();
  }
  const elapsed = performance.now() - start;
  const perOp = (elapsed / iterations) * 1000; // µs
  console.log(`📊 ${name}: ${elapsed.toFixed(2)}ms total, ${perOp.toFixed(2)}µs/op`);
  return { elapsed, perOp };
}

// Setup: Create store with 1k vectors (128D)
console.log('📦 EXPORT BENCHMARKS\n');

const store1k = new VectorStore(128, 'cosine');
for (let i = 0; i < 1000; i++) {
  const v = new Float32Array(128);
  for (let j = 0; j < 128; j++) {
    v[j] = Math.sin((i + j) * 0.001);
  }
  store1k.insert(BigInt(i), v);
}

const exportResult = measure('export_1k_128d', () => {
  store1k.export_to_bytes();
}, 100);

// Get bytes for import benchmark
const bytes1k = store1k.export_to_bytes();
console.log(`   Data size: ${(bytes1k.length / 1024).toFixed(2)} KB`);

console.log('\n📥 IMPORT BENCHMARKS\n');

const importResult = measure('import_1k_128d', () => {
  VectorStore.import_from_bytes(bytes1k);
}, 100);

console.log('\n🔄 ROUNDTRIP BENCHMARKS\n');

measure('roundtrip_1k_128d', () => {
  const exported = store1k.export_to_bytes();
  VectorStore.import_from_bytes(exported);
}, 50);

// Large dataset: 10k vectors (768D)
console.log('\n🧠 LARGE DATASET (10k x 768D)\n');

const store10k = new VectorStore(768, 'cosine');
for (let i = 0; i < 10000; i++) {
  const v = new Float32Array(768);
  for (let j = 0; j < 768; j++) {
    v[j] = Math.sin((i + j) * 0.001);
  }
  store10k.insert(BigInt(i), v);
}

const export10kResult = measure('export_10k_768d', () => {
  store10k.export_to_bytes();
}, 10);

const bytes10k = store10k.export_to_bytes();
console.log(`   Data size: ${(bytes10k.length / 1024 / 1024).toFixed(2)} MB`);

const import10kResult = measure('import_10k_768d', () => {
  VectorStore.import_from_bytes(bytes10k);
}, 10);

// Calculate throughput
const exportThroughput = (10000 * 768 * 4) / (export10kResult.elapsed / 10) / 1024 / 1024 * 1000;
const importThroughput = (10000 * 768 * 4) / (import10kResult.elapsed / 10) / 1024 / 1024 * 1000;

console.log(`\n   Export throughput: ${exportThroughput.toFixed(0)} MB/s`);
console.log(`   Import throughput: ${importThroughput.toFixed(0)} MB/s`);

console.log('\n============================================================');
console.log('✅ Persistence benchmarks complete!\n');

// Cleanup
store1k.free();
store10k.free();
