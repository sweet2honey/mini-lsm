# Mini-LSM Study Log

> **For the agent at session start:** read this file first. It holds the decision ledger,
> completed checkpoints, invariants, and the next open question. Restore context from it,
> then continue the student-owned design protocol. Never read `../mini-lsm/` or
> `../mini-lsm-mvcc/` reference implementations (AGENTS.md).

## Progress

- [x] Week 1, Day 1 — Memtable (ordered in-memory state) — `cargo x scheck` clean, 6/6 tests
- [ ] Week 1, Day 2 — Merge iterator + memtable iterator (`StorageIterator`, `MemTableIterator`)
- [ ] Week 1, Day 3 — Block
- [ ] Week 1, Day 4 — SST
- [ ] Week 1, Day 5 — Read path
- [ ] Week 1, Day 6 — Write path
- [ ] Week 1, Day 7 — SST optimizations (bloom filter)
- [ ] Week 2 — Compaction, manifest, WAL
- [ ] Week 3 — MVCC, txn, snapshot read

**Current position:** Day 1 complete. Next open question is Day 2's range-scan bound handling.

---

## Week 1, Day 1 — Memtable

### Decision ledger

Decision | Student's choice | Invariant/evidence | Consequence
---------|------------------|--------------------|------------
Tombstone value at MemTable::get | A: return raw stored value (Some(b"") for tombstone) | Engine must distinguish "deleted" from "never held" to stop probing at tombstone | MemTable::get returns Option<Bytes> verbatim; tombstone meaning decided by engine, not memtable
Put overwrite semantics | derived course rule: overwrite existing key | Book: "a single memtable never contains multiple entries for one key"; SkipMap is unique-key map | SkipMap::insert overwrites naturally; one version per key per memtable
Lock for memtable access | read lock on state, even for put/delete | SkipMap::insert takes &self; crossbeam-skiplist is lock-free concurrent; book states read lock suffices | put/delete/get all take state.read(); memtable mutation needs no exclusive lock
Tombstone at engine get | derived course rule: empty value => None | Book: "get should recognize the tombstone and report the key does not exist" | Engine get maps Some(b"") to None; single-memtable case is special case of stop-at-tombstone rule
Freeze race prevention | state_lock + recheck condition | Book: "serialize all state modifications with state_lock and recheck the condition after acquiring it" | Only the first thread that finds oversized freezes; second thread sees fresh small memtable and skips
Size accounting | count key+value len on every put, even overwrites | Book: "you may count both writes even though the skiplist retains only the latest value"; estimate >= true size | Over-count only causes early freezes; soft limit makes early freezes safe
Read path probe order | mutable first, then imm_memtables front-to-back (newest to oldest) | Book: "probe the memtables from newest to oldest"; imm_memtables stored newest-to-oldest | First memtable holding the key decides; tombstone stops probing

### Files changed
- `src/mem_table.rs` — `create` (empty SkipMap/id/size 0/wal=None), `get` (raw Option<Bytes>), `put` (SkipMap::insert + size accounting via Relaxed atomic).
- `src/lsm_storage.rs` — `get` (multi-memtable probe newest-to-oldest, tombstone->None), `put` (read lock + capacity check + state_lock recheck + freeze), `delete` (put empty value), `force_freeze_memtable` (new memtable via next_sst_id, write lock, CoW snapshot swap, insert old at imm[0]).

### Key invariants the code relies on
- SkipMap keys unique and ordered => insert overwrites => at most one value per key per memtable.
- `Bytes::clone` is a refcount increment, not a deep copy — `get` borrows the entry and produces a new refcount handle to the same bytes.
- `state.read()` derefs to `&LsmStorageState`; `.clone()` on the snapshot clones the `Arc` cheaply and releases the lock before mutation.
- `state_lock` serializes structural changes; recheck after acquiring reflects post-freeze reality.
- `imm_memtables` ordered newest-to-oldest; `insert(0, ...)` preserves it; mutable probed first.
- `approximate_size` is an upper bound; over-count only triggers early freezes (safe under soft limit).

### Boundary cases the supplied tests may not establish
1. Tombstone at memtable layer — Task 1 tests check get/overwrite but likely not `put("a","")` then `get("a")`; tombstone meaning is an engine concern verified only at Task 4.
2. Concurrent freeze race — the suite is sequential; the `state_lock`+recheck preventing double-freeze of a fresh empty memtable is untested (book: "test suite does not cover this concurrent scenario").
3. Write to frozen memtable — a thread holding an old snapshot's `Arc<MemTable>` could mutate a now-immutable memtable; tests don't construct this race.

### Commands run
- `cargo check -p mini-lsm-starter --lib` — clean.
- `cargo x copy-test --week 1 --day 1` — copied harness + week1_day1 modules.
- `cargo x scheck` (repo root) — fmt clean, check clean, 6/6 tests pass, clippy clean.

### Next unresolved design question
Day 2: memtable iterator (`MemTableIterator`, the `#[self_referencing]` struct) + `StorageIterator` trait. First decision: how `scan(lower, upper)` handles `Bound::Included` vs `Bound::Excluded`, and whether the iterator stops or signals end at the upper bound.
