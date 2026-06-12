/**
 * agent_memory.ts — Agent memory over the VelesDB TypeScript SDK.
 *
 * Exercises the `AgentMemoryClient` facade returned by `db.agentMemory()`:
 *   storeFact / searchFacts        -> semantic memory
 *   recordEvent / recallEvents     -> episodic memory
 *   learnProcedure / recallProcedures -> procedural memory
 *
 * The SDK talks to a running `velesdb-server`, so this is an illustrative
 * client (run the server first), mirroring `examples/python_example.py`.
 *
 * Run:
 *   # Terminal 1: start the server
 *   velesdb-server --data-dir ./data
 *   # Terminal 2:
 *   npm install @wiscale/velesdb-sdk
 *   npx tsx examples/agent_memory/agent_memory.ts
 */
import { VelesDB } from '@wiscale/velesdb-sdk';
import type {
  SemanticEntry,
  EpisodicEvent,
  ProceduralPattern,
} from '@wiscale/velesdb-sdk';

const DIM = 64;
const SEMANTIC = 'agent_semantic';
const EPISODIC = 'agent_episodic';
const PROCEDURAL = 'agent_procedural';

/** Deterministic, network-free embedding — hash tokens into fixed buckets. */
function fakeEmbed(text: string): number[] {
  const vec = new Array<number>(DIM).fill(0);
  for (const token of text.toLowerCase().split(/\s+/).filter(Boolean)) {
    let hash = 2166136261;
    for (let i = 0; i < token.length; i++) {
      hash ^= token.charCodeAt(i);
      hash = Math.imul(hash, 16777619);
    }
    vec[Math.abs(hash) % DIM] += 1;
  }
  const norm = Math.sqrt(vec.reduce((acc, v) => acc + v * v, 0));
  return norm > 0 ? vec.map((v) => v / norm) : vec;
}

async function main(): Promise<void> {
  const db = new VelesDB({ backend: 'rest', url: 'http://localhost:8080' });
  await db.init();

  // Each subsystem is just a cosine collection. The SDK does not create them
  // for you, so provision them up front with the embedder's dimension.
  for (const name of [SEMANTIC, EPISODIC, PROCEDURAL]) {
    await db.createCollection(name, { dimension: DIM, metric: 'cosine' });
  }

  const memory = db.agentMemory({ dimension: DIM });

  // --- Semantic: store durable facts, recall the closest to a question ---
  const fact: SemanticEntry = {
    id: 1,
    text: 'Paris is the capital of France',
    embedding: fakeEmbed('Paris is the capital of France'),
  };
  await memory.storeFact(SEMANTIC, fact);
  const facts = await memory.searchFacts(SEMANTIC, fakeEmbed('capital of France'), 3);
  console.log('Semantic recall:', facts.map((r) => r.payload?.content ?? r.payload));

  // --- Episodic: record a turn, recall it by similarity ---
  const event: EpisodicEvent = {
    eventType: 'user_message',
    data: { text: 'user asked about French geography' },
    embedding: fakeEmbed('user asked about French geography'),
  };
  const eventId = await memory.recordEvent(EPISODIC, event);
  console.log('Recorded episodic event id:', eventId);
  const events = await memory.recallEvents(EPISODIC, fakeEmbed('tell me about France'), 3);
  console.log('Episodic recall:', events.length, 'event(s)');

  // --- Procedural: learn a skill, then recall it ---
  const skill: ProceduralPattern = {
    name: 'answer_geography',
    steps: ['look up the fact', 'compose a short reply'],
    embedding: fakeEmbed('answer a geography question'),
  };
  const skillId = await memory.learnProcedure(PROCEDURAL, skill);
  console.log('Learned procedure id:', skillId);
  const procedures = await memory.recallProcedures(
    PROCEDURAL,
    fakeEmbed('answer a geography question'),
    3,
  );
  console.log('Procedural recall:', procedures.length, 'pattern(s)');

  console.log('\nAgent memory loop complete.');
}

main().catch((err) => {
  console.error('agent_memory example failed:', err);
  process.exit(1);
});
