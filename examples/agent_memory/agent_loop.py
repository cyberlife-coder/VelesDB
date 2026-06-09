#!/usr/bin/env python3
"""agent_loop.py — End-to-end AI agent memory loop on VelesDB.

Runs the three memory subsystems an autonomous agent needs, with NO API key
and NO network: the embedder below is a deterministic hash-based fake, so this
file doubles as a self-contained smoke test. It must print its trace and exit 0.

    SemanticMemory   long-term knowledge facts (store / query)
    EpisodicMemory   the timeline of what happened (record / recent / recall)
    ProceduralMemory learned skills with confidence (learn / recall / reinforce)

It also exercises the operational lifecycle: namespaced TTL + auto_expire, and a
snapshot save / load round-trip.

Run:
    cd crates/velesdb-python && maturin develop && cd -
    python examples/agent_memory/agent_loop.py
"""
import shutil
import tempfile
import time

import velesdb

DIM = 64


def fake_embed(text: str) -> list[float]:
    """Deterministic, network-free embedding.

    Hashes each token into a fixed bucket so semantically related sentences
    (which share words) land near each other under cosine similarity. Good
    enough to make recall meaningful in a reproducible smoke test; it is NOT a
    real model — swap in your embedder of choice in production.
    """
    vec = [0.0] * DIM
    for token in text.lower().split():
        bucket = hash(token) % DIM
        vec[bucket] += 1.0
    norm = sum(v * v for v in vec) ** 0.5
    return [v / norm for v in vec] if norm > 0.0 else vec


def main() -> int:
    workdir = tempfile.mkdtemp(prefix="velesdb_agent_")
    snapshot_dir = tempfile.mkdtemp(prefix="velesdb_agent_snap_")
    try:
        db = velesdb.Database(workdir)
        memory = db.agent_memory(dimension=DIM, snapshot_dir=snapshot_dir)
        print(f"AgentMemory ready (dimension={memory.dimension})\n")

        run_semantic(memory)
        run_episodic(memory)
        run_procedural(memory)
        run_ttl_cycle(memory)
        run_snapshot_cycle(memory)

        print("\nAgent loop complete.")
        return 0
    finally:
        shutil.rmtree(workdir, ignore_errors=True)
        shutil.rmtree(snapshot_dir, ignore_errors=True)


def run_semantic(memory) -> None:
    """Store durable facts, then recall the one closest to a question."""
    print("== Semantic memory: store / query ==")
    facts = [
        (1, "Paris is the capital of France"),
        (2, "The Eiffel Tower is located in Paris"),
        (3, "Rust is a systems programming language"),
    ]
    for fact_id, content in facts:
        memory.semantic.store(fact_id, content, fake_embed(content))

    question = "What is the capital of France?"
    hits = memory.semantic.query(fake_embed(question), top_k=2)
    print(f'  Q: "{question}"')
    for h in hits:
        print(f"    score={h['score']:.3f}  {h['content']}")
    print()


def run_episodic(memory) -> None:
    """Record a timeline of turns, then read it back chronologically + by similarity."""
    print("== Episodic memory: record / recent / recall ==")
    base = int(time.time())
    turns = [
        "user greeted the agent",
        "user asked about French geography",
        "agent answered about Paris",
    ]
    # Distinct, increasing timestamps so chronological ordering is well-defined.
    for offset, description in enumerate(turns):
        memory.episodic.record(
            10 + offset, description, base + offset, fake_embed(description)
        )

    recent = memory.episodic.recent(limit=2)
    print("  most recent first:")
    for ev in recent:
        print(f"    t={ev['timestamp']}  {ev['description']}")

    similar = memory.episodic.recall_similar(fake_embed("tell me about France"), top_k=1)
    print("  most similar to 'tell me about France':")
    for ev in similar:
        print(f"    score={ev['score']:.3f}  {ev['description']}")
    print()


def run_procedural(memory) -> None:
    """Learn a skill, recall it, then reinforce it and watch confidence rise."""
    print("== Procedural memory: learn / recall / reinforce ==")
    steps = ["look up the fact in semantic memory", "compose a short reply"]
    skill_emb = fake_embed("answer a geography question")
    memory.procedural.learn(20, "answer_geography", steps, skill_emb, confidence=0.5)

    before = memory.procedural.recall(skill_emb, top_k=1, min_confidence=0.0)[0]
    print(f"  recalled '{before['name']}' confidence={before['confidence']:.3f}")

    # The agent used the skill and it worked: reinforce by the recalled id.
    memory.procedural.reinforce(before["id"], success=True)
    after = memory.procedural.recall(skill_emb, top_k=1, min_confidence=0.0)[0]
    print(f"  after success      confidence={after['confidence']:.3f}")
    assert after["confidence"] > before["confidence"], "reinforce must raise confidence"
    print()


def run_ttl_cycle(memory) -> None:
    """Attach a TTL to a scratch fact, expire it, and confirm namespacing.

    TTL keys are namespaced by memory kind: a semantic id and an episodic id
    that share the same integer never cross-expire. We set a 1s TTL on semantic
    id 99 and episodic id 99, sleep past the boundary, then auto_expire.
    """
    print("== TTL + eviction: set_ttl / auto_expire ==")
    scratch = "temporary scratchpad note"
    memory.semantic.store(99, scratch, fake_embed(scratch))
    memory.episodic.record(99, "ephemeral turn", int(time.time()), fake_embed(scratch))
    memory.set_semantic_ttl(99, 1)
    memory.set_episodic_ttl(99, 1)

    # Sleep just past the 1s boundary so both entries are eligible for expiry.
    time.sleep(1.2)
    result = memory.auto_expire()
    print(f"  auto_expire -> {result}")
    assert result["semantic_expired"] == 1, "expired semantic id 99"
    assert result["episodic_expired"] == 1, "expired episodic id 99"

    # The durable facts (ids 1-3) survive: TTL only touched the scratch ids.
    survivors = {h["id"] for h in memory.semantic.query(fake_embed("France"), top_k=5)}
    assert 99 not in survivors and 1 in survivors, "TTL must not evict durable facts"
    print(f"  durable facts survived: {sorted(survivors)}")
    print()


def run_snapshot_cycle(memory) -> None:
    """Version the whole memory, mutate, then roll back to the snapshot."""
    print("== Snapshot: save / mutate / load round-trip ==")
    version = memory.snapshot()
    print(f"  saved snapshot v{version}")

    # Mutate after the snapshot: add a fact that the rollback must drop.
    memory.semantic.store(500, "this fact is added after the snapshot", fake_embed("post snapshot"))
    post = {h["id"] for h in memory.semantic.query(fake_embed("post snapshot"), top_k=5)}
    assert 500 in post, "the post-snapshot fact is present before rollback"

    restored = memory.load_latest_snapshot()
    after = {h["id"] for h in memory.semantic.query(fake_embed("post snapshot"), top_k=5)}
    assert 500 not in after, "rollback must drop the post-snapshot fact"
    print(f"  loaded snapshot v{restored}; post-snapshot fact id 500 gone after rollback")
    print(f"  versions on disk: {memory.list_snapshot_versions()}")


if __name__ == "__main__":
    raise SystemExit(main())
