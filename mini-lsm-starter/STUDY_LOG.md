# Mini-LSM Study Log

> **For the agent at session start:** read this file first. It holds the decision ledger,
> completed checkpoints, invariants, and the next open question. Restore context from it,
> then continue the student-owned design protocol. Never read `../mini-lsm/` or
> `../mini-lsm-mvcc/` reference implementations (AGENTS.md).

## Progress

- [x] Week 1, Day 1 — Memtable (ordered in-memory state) — 6/6 tests
- [x] Week 1, Day 2 — Iterators (memtable iterator, merge iterator, LSM iterator, fused iterator, engine scan) — 14/14 tests cumulative
- [x] Week 1, Day 3 — Block (builder + encode/decode + iterator, binary-search seek) — 23/23 tests cumulative
- [x] Week 1, Day 4 — SST (builder, meta encode, iterator, block cache) — 30/30 tests cumulative
- [ ] Week 1, Day 5 — Read path
- [ ] Week 1, Day 6 — Write path
- [ ] Week 1, Day 7 — SST optimizations (bloom filter)
- [ ] Week 2 — Compaction, manifest, WAL
- [ ] Week 3 — MVCC, txn, snapshot read

**Current position:** Day 4 complete. Next open question is Day 5's read path (multi-source merge across memtables + SSTs, layered iterator tree).

---

## Week 1, Day 4 — SST

### Decision ledger

Decision | Student's choice | Invariant/evidence | Consequence
---------|------------------|--------------------|------------
Block cut in SsTableBuilder::add | inner `BlockBuilder::add` false -> `std::mem::replace` builder with fresh builder, build+encode old, push meta {offset = data.len() BEFORE append, first_key, last_key}, append block bytes, re-offer key to fresh builder | book: "split a new block when the current block is full"; fresh BlockBuilder ALWAYS accepts its first entry (Day 3 carve-out) so add never fails | only the last block may be smaller than block_size
Meta layout on disk | delegated: count-prefixed `u32 num_blocks LE`, then N records of `u32 offset, u16 first_len, first_key, u16 last_len, last_key` | book: meta section must be self-describing; reserve exact size up-front (buf.reserve(estimated)) | decode is seek-capable without trailing sentinel; consistent LE with Block from Day 3
find_block_idx strategy | first-key-only binary search: `partition_point(\|meta\| meta.first_key <= key).saturating_sub(1)`; iterator advances one block if in-block lower-bound lands invalid | book recommendation; student walked `seek(d)` with blocks [a,b,c],[e,f,g] -> picks block 1 -> invalid -> advance to block 2 on `e` | one extra block read possible per gap key
Field width constants | OFFSET_ENCODED_LEN (u32), LENGTH_ENCODED_LEN (u16), BLOCK_META_ENTRY_OVERHEAD = OFFSET + 2*LENGTH | user: derive from encoded widths, not host word size | all paths (encode/decode/open) express widths symbolically; offsets are u32 NOT usize
Endianness of meta & footer | all LE, consistent with Block | **bug caught by test_sst_decode**: build wrote footer with put_u32_le, open read with get_u32 (BE default) -> meta_offset read as huge value -> subtraction overflow at `len - 4 - meta_offset` | student diagnosis was immediate; never mix byte orders across encode/decode

### Files changed
- `src/table/builder.rs` — `new`, `add` (sorted assert, finish_block on full), `estimated_size` (data bytes only), `build` (footer = u32 LE meta_offset, FileObject::create).
- `src/table.rs` — OFFSET/LENGTH width constants, `BlockMeta::encode/decode` (count-prefixed, reserve exact), `SsTable::open`, `read_block` (offset->offset+1 or meta_offset, decode), `read_block_cached` (moka `try_get_with` on (sst_id, block_idx), Arc<Error> for shared misses), `find_block_idx` (partition_point).
- `src/table/iterator.rs` — SsTableIterator with `advance_if_needed` (single-hop to next block on invalid); all seeks/next funnel through it.

### Key invariants the code relies on
- BlockMeta records appear in same key order as data blocks; meta.offset = where block bytes begin.
- Block i spans [meta[i].offset, meta[i+1].offset); last block up to meta_offset. No explicit block lengths.
- Lower-bound seeks never skip a key >= target: if block i's iterator lands invalid, ALL keys in block i < key, AND meta[i+1].first_key > key (because find_block_idx found the LAST block with first_key <= key) -> single advance suffices. No loop needed because blocks are non-empty and strictly ordered.
- Block cache key MUST be (sst_id, block_idx); block_idx alone collides ACROSS SSTs.

### Boundary cases the supplied tests may not establish
1. **read_block_cached cache-hit/miss-coalescing path is COMPLETELY UNTESTED** — `build_for_test` passes `block_cache: None` in every test, so `try_get_with` (concurrent miss coalescing) never runs. The `Some(cache)` branch is dead code to the suite.
2. **Repeated seek_to_key to an EARLIER position on the same iterator** — suite only seeks forward per offset pass. Works because seek_to_key rebuilds the block iterator from scratch.
3. **Eviction under live Arc handles** — an iterator holds Arc<Block> outside the cache; eviction removes the map key, not the memory. Cache capacity bounds cache-held blocks only.
4. **Blocks larger than block_size** — generator's block_size=128 entries ~23 bytes uniform; first-entry carve-out (one oversized entry) never hit in SST context.
5. **Blocks with 0 keys** — advance_if_needed does one hop; would skip a needed second hop if an empty block could exist. Relies on blocks-always-non-empty.

### Commands run (Day 4)
- `cargo check --lib` — clean first pass after one BufMut import fix and one garbled-edit repair.
- `cargo x copy-test --week 1 --day 4` — added week1_day4; prior days retained (copy-test is additive when files already exist).
- `cargo test week1_day4` — FIRST RUN FAILED: `test_sst_decode` panicked "attempt to subtract with overflow" at table.rs:155 (`len - 4 - meta_offset as u64` where meta_offset was huge). **Student diagnosis**: endianness mismatch — builder wrote footer with put_u32_le, open read with get_u32 (big-endian default in `bytes`). Fix: `get_u32_le`.
- Second run — 7/7 pass.
- `cargo x scheck` (repo root) — fmt clean, check clean, clippy clean, **30/30 tests** (6 day1 + 8 day2 + 9 day3 + 7 day4).

### Review lines (Day 4)
1. `partition_point(\|meta\| meta.first_key <= key).saturating_sub(1)` — Q: what does partition_point return, what does -1 produce, what happens on `seek("")`? A: "points to the first element NOT satisfying the predicate; sub 1 gives last block with first_key <= search key" — correct. (seek("") case walked: partition_point=0, saturating_sub=0 -> block 0 = correct lower-bound landing spot.) Verdict: correct.
2. `advance_if_needed` — Q: prove a single hop always suffices; name both invariants. A(student): "partition_point ensures + BlockBuilder never emits an empty block, so hopping always lands on a valid key." Correct — (a) blocks non-empty => seek_to_first always valid; (b) find_block_idx picks the LAST block with first_key <= key => every key in block i+1 is > target, so its first entry is exactly the lower bound. Verdict: correct after one nudge (initial answer named only the search half).
3. Block-read path (book checkpoint) — Q: when does disk IO happen; what does the iterator retain vs the cache? First answer "IO happens when reading block 4, cache retains 1~4" — partial (conflated iterator and cache lifetimes, missed miss-vs-hit split). Corrected after framing: "`!self.blk_iter.is_valid()` triggers the new read (maybe from cache)" — `try_get_with` decides disk vs cheap Arc clone on the miss. Corrected full picture the student was walked to: iterator retains exactly ONE Arc<Block> (current position); cache retains every block read so far up to capacity, independent of iterator; eviction never invalidates a held Arc and iterator-drop never prunes the cache. Verdict: correct after one correction.

## Week 1, Day 3 — Block

### Decision ledger

Decision | Student's choice | Invariant/evidence | Consequence
---------|------------------|--------------------|------------
Size gate in `add` | `projected = S + 6 + key_len + value_len` (S = current full encoded size); reject (`false`, no mutation) when `projected > block_size`; first entry always accepted | book invariant 4 + forward-progress carve-out | block never exceeds target unless one entry alone exceeds it; builder never stalls on an oversized first key
Endianness of u16 fields | little-endian (`put_u16_le`/`get_u16_le`, manual `from_le_bytes` in iterator) | native byte order on x86 (no swap on hot path); encoder/decoder must agree; byte-comparison tests confirm immediately | all lengths, offsets, and count are LE
`seek_to_key` landing (course rule, derived) | first key >= target, else invalid; below-first -> first entry | book invariant 5 + example (seek 2 lands on 3) | lower-bound semantics at all edges
End-of-block behavior | `next()` off the end -> key empty -> invalid; no panic; crossing to the next block is the future SSTIterator's job | book: "caller can then move to another block"; BlockIterator holds one `Arc<Block>`, no next-block reference | single-block scans terminate cleanly; layer discipline matches day-2 tombstone-above-merge
`seek_to_key` algorithm | binary search over entry indices, probing `offsets[mid]` and decoding only key_len+key (never value) | book: offsets "turn variable-length entries into an index" | O(log n) probes x O(K) key compare each
Cursor representation | copy current key into `KeyVec`; keep value as `(start,end)` range into data section | book Task 2: copy key now so the struct survives future key compression; value stays zero-copy | value() is a raw slice; struct unchanged when compression lands

### Files changed
- `src/block/builder.rs` — `new`, `add` (record offset, append `key_len|key|value_len|value` LE, first-entry carve-out), `is_empty`, `build`, plus `estimated_size()` helper.
- `src/block.rs` — `encode` (`data | offsets | num_of_elements`), `decode` (count from tail, offsets section before it, data section before that; trusts input).
- `src/block/iterator.rs` — shared private `seek_to(idx)` decoder; binary-search `seek_to_key`; `next` = idx+1 then seek_to; validity = non-empty key.

### Key invariants the code relies on
- encode/decode are exact inverses for every valid block; the byte-comparison tests pin field order and endianness.
- One ordered offset per entry, each a valid position in the data section; footer = offsets + u16 count.
- `is_valid` (non-empty key) is the sole termination gate; exhaustion clears the key.
- The decoder TRUSTS its input: it indexes with no validation of count or offset sanity (book checkpoint flags this).
- Keys non-empty (debug_assert); empty value (tombstone) is preserved verbatim by builder and iterator.

### Boundary cases the supplied tests may not establish
1. **Tombstone through the block layer** — `add("a", [])`: value_len=0; iterator must return `value() == b""` and stay valid. Supplied tests use only non-empty values.
2. **Decoder on malformed bytes** — wrong count / out-of-range offset / truncated length slice: our decoder panics on out-of-bounds indexing (book checkpoint asks what a production decoder would validate).
3. **Duplicate keys inside one block** — the builder never dedupes; binary-search lower bound lands on the FIRST duplicate. (Book asks: can a block contain duplicated keys? Format allows; upper layers assume not.)
4. **u16 length overflow** — key or value > 65535 would truncate silently; only debug_assert guards it.

### Commands run (Day 3)
- `cargo check --lib` — clean on the first pass.
- `cargo x copy-test --week 1 --day 3` — copied harness + week1_day3, and left `src/tests/` with ONLY those two files (day1/day2 files dropped, same copy-test behavior noted on Day 2).
- `cargo test week1_day3` — 9/9 pass (encode, decode, build_single_key, build_full, build_large_1/2, build_all, iterator, seek_key).
- `cargo x copy-test --week 1 --day 1` and `--day 2` — restored day1/day2; copy-test is additive per day and regenerates tests.rs from all files present.
- `cargo x scheck` (repo root) — fmt clean, check clean, **23/23 tests pass: 6 day1 + 8 day2 + 9 day3** (Day 2's log entry has the 6/8 split transposed; total was always 14), clippy clean.

### Review lines (Day 3)
1. `if !self.is_empty() && self.estimated_size() + entry_size > self.block_size { return false; }` — Q(a): what does `!self.is_empty()` protect, what breaks if deleted? A: "big entry causes storage to stall" — refined: removing it makes `add` return false on an EMPTY builder -> SST builder emits nothing, opens another empty block, retries forever; the carve-out = guaranteed forward progress. Q(b): is the count field a size or value claim; why is 6 the right per-entry cost? A: "size claim; cost = key_len + value_len + entry_offset" — exactly: `num_of_elements` is always 2B regardless of value, so per-entry cost = 2+2+2 = 6. Verdict: both correct.
2. `len` checks in `decode`/`seek_to` — Q: name two distinct corrupt-block failure modes and what a production decoder validates first. First attempt (wrong): "indices only" — caught count-lie underflow but mistook safe-Rust slicing for C-style OOB reads. Correction given: safe Rust panics on slice bounds, never slow/large reads; and a wrong-but-in-bounds count causes SILENT mis-slicing (valid-looking, wrong block — data tail eaten into offsets). Q follow-up (hint offered): ordered validation sequence. Student's answer: "add a crc field in the final block covering data+offset+num" — the RocksDB mechanism (1B compression-type + 4B CRC32C trailer). Supplied the remaining sequence: CRC hash first; buffer >= footer; `count*2` fits; offsets monotonic-and-bounded; lengths bounded per entry before slicing. Key teaching point the student owns: CRC catches *random* corruption, structure checks catch *malice* (checksum is forgeable — integrity, not security). Verdict: correct after one correction + one hint.
3. Tombstone round-trip (`add("a", [])`) — Q: what does encode emit, what does value() return, is it valid, why load-bearing? A: key_len=1, "a", value_len=0, empty value; value() == b""; iterator stays valid because validity tracks cursor not value. Correct. Consequence (supplied): if empty-value implied invalid, scans would truncate at tombstones — same bug class as day-2 LsmIterator, at the storage layer. Verdict: correct.
4. Final understanding — duplicate keys (`add("k","v1")` then `add("k","v2")`) — Q: what does the block hold, what does seek return, where does "no duplicates" live, consequence if violated? First attempt (WRONG): student conflated the block with the SkipMap ("newer overwrites", "enforced in code through skip map") — the block is a byte array; `BlockBuilder::add` has no lookup at all, both entries land in `data` with two offsets. Corrected: block holds BOTH k->v1 (idx 0) and k->v2 (idx 1); binary search keeps shrinking `high` on `>= target` so it lands on the FIRST matching entry -> seek returns v1 (the OLDER value) -> silent staleness for any caller that violates the convention. "No duplicates" is caller-discipline, not a block property: enforced upstream by SkipMap (memtable) and later by the SST builder's one-pass write; within-block dups across levels are handled by merge iterators, not here. Verdict: WRONG first, correct after the byte-array correction.


## Week 1, Day 2 — Iterators

### Decisions carried in (from Day 1, unchanged)
- Tombstone = empty value; memtable returns it verbatim, engine interprets.
- imm_memtables newest-to-oldest; mutable probed first.
- state_lock + recheck for freeze races; size accounting is an upper bound (soft limit).

### Slice 1 — MemTableIterator

Decision | Student's choice | Invariant/evidence | Consequence
---------|------------------|--------------------|------------
Iterator exhaustion signal | empty-key sentinel in `item` | next() writes (Bytes::new(), Bytes::new()) when Range runs out; book: ok next() need not mean valid | one piece of state; a real empty key would be misreported as exhausted — accepted simplification
Upper-bound handling in scan | delegated (choose for me): pass bounds straight through via map_bound | SkipMap::range natively respects Included/Excluded | no manual bound adjustment; exclusion enforced by SkipMap, not our code

Files: `src/mem_table.rs` — `scan` via ouroboros builder (clone Arc<SkipMap>, build range iterator borrowing it, position at first entry); `key`/`value`/`is_valid`/`next` implemented.

### Slice 2 — MergeIterator

Decision | Student's choice | Invariant/evidence | Consequence
---------|------------------|--------------------|------------
Duplicate resolution | newest (lowest-index) child wins; next() must advance EVERY heap child at the emitted key | book's merged example; starter Ord: key cmp, then index, reversed (max-heap pops smallest) | otherwise stale duplicate resurfaces after the surfaced child advances
Error path in next() | delegated: pop errored child from heap BEFORE returning Err | PeekMut::drop re-sorts heap, would read key() on poisoned child | comparator never touches a poisoned iterator

Files: `src/iterators/merge_iterator.rs` — `create` pushes only valid children; `next` snapshots emitted key, drains all heap children at that key (pop exhausted/errored), advances and reseats `current`.

### Slice 3 — LsmIterator + FusedIterator

Decision | Student's choice | Invariant/evidence | Consequence
---------|------------------|--------------------|------------
Where tombstones are filtered | AFTER duplicate resolution, in LsmIterator above the merge | filtering at MemTableIterator level would lose precedence info and resurrect older values (e.g. b->2 after b->delete) | tombstones visible through merge; hidden only at engine wrapper
FusedIterator semantics | course contract (starter doc comment) | invalid -> next no-op; error -> is_valid false forever, next always errs | mechanical

Files: `src/lsm_iterator.rs` — `LsmIterator::new` eager-skips tombstones (after merge may start on one); `next` advances then skips; `key` returns `raw_ref()` (KeyType = &[u8], not KeySlice). `FusedIterator` latches `has_errored`.

### Slice 4 — engine `scan`

Decision | Student's choice | Invariant/evidence | Consequence
---------|------------------|--------------------|------------
Merge constructor order | mutable first (index 0), then imm_memtables front-to-back | same precedence as get probe order from Day 1 | newest-wins on ties via index tie-break in MergeIterator's Ord

Files: `src/lsm_storage.rs` — `scan` snapshots state, builds one MemTableIterator per memtable, wraps MergeIterator -> LsmIterator -> FusedIterator. Added imports for MergeIterator and MemTableIterator.

### Invariants
- Only valid children in heap (comparator reads key()).
- `current` is always the minimum (key, index) across children.
- After every `next()`, exhausted/errored children removed before PeekMut guard drop.
- `next()` returning Ok does NOT imply still valid (empty memtable starts invalid).
- Scan reflects the snapshot at creation time; concurrent writes to the same memtable are visible only to new scans (open question for the book's bonus experiment).

### Boundary cases the supplied tests may not establish
1. **Three iterators, same key in all three** — next() must drain it from two heap children plus `current`. Suite likely only tests two-iterator merges.
2. **Empty keys via `put(b"", b"1")`** — MemTableIterator's empty-key sentinel would treat it as exhaustion; a misbehavior we accept as a simplification (defensible only if empty keys are disallowed).
3. **Scan-time concurrent put** — MemTableIterator holds `Arc<SkipMap>`; does it see keys inserted after `scan()` was created? (book's suggested experiment — stability is unclear from the type signature alone.)
4. **get() vs scan() on same data** — get and scan must agree with respect to tombstones and freeze boundaries.

### Commands run (Day 2)
- `cargo x copy-test --week 1 --day 2` — copied harness + week1_day2.
- `cargo x copy-test --week 1 --day 1` — re-copied after tests.rs was overwritten (copy-test regenerates tests.rs from whatever test files exist in the target dir; day-2 copy had dropped day-1's module registration).
- `cargo x scheck` (repo root) — fmt clean, check clean, **14/14 tests pass** (8 day1 + 6 day2), clippy clean.

### Review lines (Day 2)
1. `!self.borrow_item().0.is_empty()` (MemTableIterator::is_valid) — Q: why is an empty key a valid exhaustion signal; what breaks on a real empty key? A: sentinel is single-source-of-truth; real empty key would be misread as exhausted (accepted limitation). Verdict: answered correctly.
2. `self.1.key().cmp(&other.1.key()).then(self.0.cmp(&other.0)).reverse()` (HeapWrapper Ord, starter-provided but relied on) — Q: why does `.reverse()` make small keys pop first; why ascending-index tie-break and who orders the vector? A: max-heap + reversed cmp surfaces smallest key; lower index = newer child placed by `scan`/merge constructor. Verdict: correct after terminology fix (memtables, not SSTs, at this stage).
3. `self.inner.next()?; self.skip_tombstones()` (LsmIterator::next) — Q: why advance twice; what breaks if `is_valid` just returned false on tombstones? A: first advance moves past the current entry, skip drains the tombstone chain; without it, a tombstone would *terminate* the caller's loop and truncate everything after it (e.g. `c->4` after `b->delete`). Key insight (student): tombstone is a normal entry at the cursor level; "deleted" exists only at the LsmIterator layer. Verdict: correct.
4. No separate review line for Slice 4 (engine `scan` — implemented under a continue window); the adversarial three-iterator duplicate-drain (`b->delete` over `b->2`, `b->1` + `a->4`, `c->3` -> output `a->4, c->3`) served as its check. Verdict: student traced it correctly.

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
- `Bytes::clone` is a refcount increment, not a deep copy.
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


### Review lines (Day 1)
1. `self.map.get(key).map(|entry| entry.value().clone())` (MemTable::get) — Q: what does entry.value().clone() produce; what breaks if value() without clone, or if None on empty value? A: new refcount handle to same bytes (not a move-out; map entry stays); returning None for tombstones would erase the deleted-vs-never-held distinction. Verdict: correct after refinement (clone ≈ Arc bump, entry untouched).
2. `let memtable = self.state.read().memtable.clone();` (engine get/put) — Q: what is cloned, why release the lock at the statement end; what breaks with `&self.state.read().memtable` (no clone), and with holding the read lock across `memtable.put`? A: Arc refcount bump, not map copy; bare reference = dangling after the guard (temporary) drops, borrow checker rejects; holding the lock across put serializes writers and blocks freeze's write lock. Verdict: part 1 correct; part 2 needed explanation (temporary-guard lifetime + freeze write-lock conflict).
3. `std::mem::replace(&mut snapshot.memtable, new_memtable); snapshot.imm_memtables.insert(0, old_memtable);` (freeze) — Q: why replace vs plain assignment; what does insert(0) guarantee? A: replace yields the old Arc instead of dropping it (plain assignment would lose the handle); insert at 0 preserves newest-to-oldest so `imm.first()` is the most recently frozen. Verdict: correct after fix — initially read insert(0) as "queried later"; it's queried FIRST.
4. `std::iter::once(&snapshot.memtable).chain(snapshot.imm_memtables.iter())` (engine get probe order) — Q: what does the chain produce; what breaks if order is swapped (imm first, mutable last)? A: mutable (newest) then imm newest-to-oldest; swap makes an older imm hit first and shadow the newer mutable entry (stale a->1 over a->3; tombstone-overwrite case resurrects a deleted key). Verdict: correct after concrete walkthrough.