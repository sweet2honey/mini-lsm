# The Confusion: Reversed Ordering

Your `Ord` implementation ([lines 44-51](merge_iterator.rs#L44-L51)):
```rust
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // This would yield the 'smallest' key
        self.1
            .key()
            .cmp(&other.1.key()) // Compare keys
            .then(self.0.cmp(&other.0)) // Break ties with index
            .reverse() // REVERSE the result!
    }
```

**With `.reverse()`:**
- If `self.key() < other.key()` → returns `Greater` (reversed!)
- If `self.key() > other.key()` → returns `Less` (reversed!)
- This makes `BinaryHeap` a **min-heap** (smallest on top)

## Why This Matters

Rust's `BinaryHeap` is always a **max-heap** - it puts the "greatest" element on top according to `Ord`. By reversing the comparison, you trick it into behaving like a min-heap:

```rust
// Without reverse: max-heap
Wrapper(key="c") > Wrapper(key="a")  // "c" goes to top

// With reverse: min-heap  
Wrapper(key="c") < Wrapper(key="a")  // "a" goes to top
```

## The Line You Asked About

```rust
if *current_wrapper < *heap_top {
    std::mem::swap(current_wrapper, &mut *heap_top);
}
```

**How the comparison works**

- `BinaryHeap` is max-oriented, but our `Ord::cmp` returns the reversed order, **so the “largest” element in heap terms is actually the smallest key/lowest index value.**
- When we compare `*current_wrapper < *heap_top`, the `<` uses the same `Ord`: with reverse ordering, `<` returns `true` if `current`’s key is greater than the heap top’s key. In that case, the heap actually holds a smaller key, so we swap to keep `current` as the minimum.

Whenever `current` is invalid after advancing, we pop from the heap and assign the next smallest iterator to `current`. If the heap is empty, `current` stays invalid and `is_valid()` will report `false`.

# A new LsmIterator

## Tombstones: How Deletion Works in LSM Trees

In an LSM tree, data exists across multiple layers (current memtable, immutable memtables, SSTables). You **cannot immediately remove** a deleted key from older layers because:
1. Older layers are immutable (imm_memtables) or on disk (SSTables)
2. Rewriting them on every delete would be too expensive

**Solution: Tombstones**

When you delete a key, the system writes it with an **empty value** (`&[]`) to the current memtable:

```rust
pub fn delete(&self, key: &[u8]) -> Result<()> {
    self.put(key, &[])  // Empty value = tombstone marker
}
```

This tombstone **shadows** older values of the same key in lower layers.

## Example: Multi-Layer State

```
┌─────────────────────────┐
│ Current Memtable (L0)   │  ← Highest priority
│  "key1" → "new_value"   │
│  "key2" → ""            │  ← TOMBSTONE (deleted)
└─────────────────────────┘
┌─────────────────────────┐
│ Imm Memtable (L1)       │
│  "key2" → "old_value"   │  ← Shadowed by tombstone above
│  "key3" → "value3"      │
└─────────────────────────┘
```

When scanning:
- MergeIterator sees **both** entries for "key2"
- It picks the L0 entry (lower index = higher priority)
- Result: "key2" → "" (empty value)

**This empty value must be hidden from users!**

## Skipping Logic in LsmIterator

### 1. Skip at Initialization

```rust
pub(crate) fn new(mut iter: LsmIteratorInner) -> Result<Self> {
    // Skip initial deleted keys
    while iter.is_valid() && iter.value().is_empty() {
        iter.next()?;
    }
    Ok(Self { inner: iter })
}
```

**Why?** If the first key in the iteration range is a tombstone, the iterator must start **after** it.

**Example:**
```rust
storage.scan(Bound::Included(b"key2"), Bound::Unbounded)
```

If "key2" is deleted (tombstone), MergeIterator positions there initially. Without the skip:
- `is_valid()` → true
- `key()` → "key2"  ← **BUG: Shows deleted key!**

With the skip:
- Detects `value().is_empty()`
- Calls `next()` → advances to "key3"
- Now correctly positioned at first **live** key

### 2. Skip During Iteration

```rust
fn next(&mut self) -> Result<()> {
    // Forward to next key
    self.inner.next()?;

    // Skip any tombstones
    while self.inner.is_valid() && self.inner.value().is_empty() {
        self.inner.next()?;
    }

    Ok(())
}
```

**Why?** Even during iteration, we might encounter tombstones.

**Example sequence:**
```
MergeIterator order: "key1"(value), "key2"(tombstone), "key3"(value)

Call next():
1. self.inner.next() → moves to "key2"
2. Sees value().is_empty() → true
3. self.inner.next() → moves to "key3"
4. Sees value().is_empty() → false
5. Stops, positioned at "key3"
```

### 3. Why Both Locations?

- **`new()`**: Handles when iteration **starts** on a tombstone
- **`next()`**: Handles when iteration **moves to** a tombstone

Both are necessary because:
- You might start scanning at a deleted key
- You might be iterating over live keys and hit deleted ones

## Invariant Maintained

**LsmIterator invariant:** When `is_valid()` returns `true`, the current key-value pair must **never** have an empty value.

```rust
// This invariant is always maintained:
if lsm_iter.is_valid() {
    assert!(!lsm_iter.value().is_empty());
}
```

Users of LsmIterator see only **live** data, deleted keys are completely invisible.

# Test Your Understanding

## 1. What is the time/space complexity of using your merge iterator?

**Time Complexity:**
- **Construction**: O(N log N) where N is the number of child iterators. Each iterator is added to the binary heap with O(log N) insertion.
- **`next()`**: O(log N) for each call. We advance one iterator, then maintain the heap with O(log N) operations (pop + push).
- **Iterating over K keys**: O(K log N) total.

**Space Complexity:**
- **Heap storage**: O(N) for the binary heap holding N iterators.
- **Current iterator**: O(1) additional space for the current wrapper.
- **Total**: O(N) where N is the number of child iterators.

## 2. Why do we need a self-referential structure for memtable iterator?

The `MemTableIterator` needs to:
1. Hold an `Arc<SkipMap>` reference to the memtable data
2. Hold a `Range` iterator that borrows from that same SkipMap

This creates a self-reference:
```rust
struct MemTableIterator {
    map: Arc<SkipMap>,
    iter: Range<'???, ...>, // Needs to borrow from map above!
}
```

Without `ouroboros`, we'd need an explicit lifetime `'a`, but then the struct can't own both the data and the reference to it. The `#[self_referencing]` macro solves this by generating safe code that maintains both.

## 3. If a key is removed (there is a delete tombstone), do you need to return it to the user? Where did you handle this logic?

**No**, tombstones must **not** be returned to users. Handled in `LsmIterator`:

**Location 1** - [`LsmIterator::new()`](lsm_iterator.rs#L34-L39):
```rust
// Skip initial deleted keys
while iter.is_valid() && iter.value().is_empty() {
    iter.next()?;
}
```

**Location 2** - [`LsmIterator::next()`](lsm_iterator.rs#L60-L67):
```rust
fn next(&mut self) -> Result<()> {
    self.inner.next()?;
    // Skip any tombstones
    while self.inner.is_valid() && self.inner.value().is_empty() {
        self.inner.next()?;
    }
    Ok(())
}
```

## 4. If a key has multiple versions, will the user see all of them? Where did you handle this logic?

**No**, the user only sees the **newest** version. Handled in `MergeIterator`:

**Location** - [`MergeIterator::new()`](merge_iterator.rs) and the ordering logic:

The `HeapWrapper` comparison uses:
```rust
self.1.key().cmp(&other.1.key())
    .then(self.0.cmp(&other.0))  // Lower index = higher priority
    .reverse()
```

When multiple iterators have the same key:
- The iterator with the **lower index** wins (earlier in construction order)
- Since we construct with: current memtable (index 0) → newer imm (index 1) → older imm (index 2)...
- The newest version always has the lowest index and is selected

When `next()` is called, we advance **only** the current iterator, automatically skipping older versions of the same key in other iterators.

## 5. If we want to get rid of self-referential structure and have a lifetime on the memtable iterator (i.e., MemtableIterator<'a>, where 'a = memtable or LsmStorageInner lifetime), is it still possible to implement the scan functionality?

**Yes**, but with significant trade-offs:

**Approach**: Change `scan()` to return an iterator with a lifetime:
```rust
pub fn scan<'a>(&'a self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) 
    -> LsmIterator<'a>
```

**Problem**: The iterator would hold a read lock or borrow on `LsmStorageInner` for its entire lifetime. This means:
- ❌ Cannot write while iterator is alive (no concurrent writes)
- ❌ Cannot freeze memtables while scanning
- ❌ Holding locks for long periods → poor concurrency

**Current approach is better**: Using `Arc<SkipMap>` allows:
- ✅ Snapshot isolation: iterator sees consistent point-in-time view
- ✅ Concurrent writes can continue
- ✅ Memtables can be frozen/flushed while iterating
- ✅ No lock contention

## 6. What happens if (1) we create an iterator on the skiplist memtable (2) someone inserts new keys into the memtable (3) will the iterator see the new key?

**No**, the iterator will **not** see the new key.

**Reason**: The `Range` iterator created from `SkipMap` is a **snapshot** at the time of creation. The skiplist uses lock-free techniques and the iterator captures the structure's state when it's created.

**From crossbeam-skiplist docs**: The iterator is based on a snapshot of the skiplist and won't see concurrent modifications.

This provides **snapshot isolation** - each scan sees a consistent view of the data as it existed when the iterator was created.

## 7. What happens if your key comparator cannot give the binary heap implementation a stable order?

**The merge iterator will break!** Problems:

1. **Heap invariant violations**: BinaryHeap relies on consistent ordering. If `a.cmp(b)` gives different results over time, the heap structure becomes corrupted.

2. **Missing or duplicate keys**: The iterator might:
   - Skip keys that should be returned
   - Return the same key multiple times
   - Return keys in wrong order

3. **Panic or undefined behavior**: Heap operations might panic or produce nonsensical results.

**Why we use index tiebreaker**: Even when keys are equal, we need a stable secondary ordering:
```rust
self.1.key().cmp(&other.1.key())
    .then(self.0.cmp(&other.0))  // Index provides stability!
```

The index ensures that `cmp()` always returns the same result for the same pair of wrappers.

## 8. Why do we need to ensure the merge iterator returns data in the iterator construction order?

**To respect LSM tree semantics**: Data priority decreases with age:

```
Priority (newest → oldest):
1. Current memtable (index 0) - newest writes
2. Imm memtable 0 (index 1) - recently frozen
3. Imm memtable 1 (index 2) - older frozen data
4. L0 SSTables (index 3+) - flushed to disk
5. Deeper levels... - progressively older
```

**Why this matters**:
- **Updates**: If key="x" was written multiple times, we must return the **latest** value
- **Deletes**: If key="x" was deleted (tombstone in newer layer), it must shadow older values
- **Consistency**: Users expect to see the most recent state

**Without respecting order**: Users would see stale data, deletions wouldn't work, and the database would be incorrect.

## 9. Is it possible to implement a Rust-style iterator (i.e., next(&self) -> (Key, Value)) for LSM iterators? What are the pros/cons?

**Not directly**, but possible with trade-offs:

**Standard Iterator Trait**:
```rust
trait Iterator {
    type Item;
    fn next(&mut self) -> Option<Self::Item>  // Note: &mut self
}
```

**LSM Iterator Challenges**:
1. **Borrowed data**: Keys/values are borrowed from internal storage
2. **Error handling**: Database I/O can fail, but `Iterator::Item` doesn't support `Result`
3. **Fallible iteration**: Need `Result<Option<(Key, Value)>>`, not `Option<(Key, Value)>`

**Possible workarounds**:

**Option A**: Return owned data (copying):
```rust
fn next(&mut self) -> Option<(Bytes, Bytes)>  // Clone keys/values
```
- ✅ Fits standard Iterator
- ❌ Expensive: copies all data
- ❌ Extra allocations

**Option B**: Use fallible iterator crate:
```rust
fn next(&mut self) -> Result<Option<(&[u8], &[u8])>>
```
- ✅ Zero-copy
- ✅ Proper error handling
- ❌ Non-standard trait
- ❌ Can't use for loops or standard iterator methods

**Current design (`StorageIterator`) is optimal**:
```rust
fn is_valid(&self) -> bool;
fn key(&self) -> &[u8];
fn value(&self) -> &[u8];
fn next(&mut self) -> Result<()>;
```
- ✅ Zero-copy access
- ✅ Explicit error handling
- ✅ Clear validity state
- ❌ Can't use standard Rust iterator idioms

## 10. The scan interface is like `fn scan(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>)`. How to make this API compatible with Rust-style range (i.e., `key_a..key_b`)?

**Use `RangeBounds` trait**:

```rust
use std::ops::RangeBounds;

pub fn scan<R>(&self, range: R) -> Result<LsmIterator>
where
    R: RangeBounds<[u8]>,
{
    let lower = match range.start_bound() {
        Bound::Included(key) => Bound::Included(key),
        Bound::Excluded(key) => Bound::Excluded(key),
        Bound::Unbounded => Bound::Unbounded,
    };
    let upper = match range.end_bound() {
        Bound::Included(key) => Bound::Included(key),
        Bound::Excluded(key) => Bound::Excluded(key),
        Bound::Unbounded => Bound::Unbounded,
    };
    // ... rest of scan logic
}
```

**Now you can use**:
```rust
storage.scan(b"a"..b"z")?;        // Excluded end
storage.scan(b"a"..=b"z")?;       // Included end
storage.scan(..)?;                 // Full range - scans everything!
storage.scan(b"prefix"..)?;       // From prefix to end
```

**What happens with `..` (full range)**:
- Both bounds are `Unbounded`
- Scans from first key to last key in database
- Useful for full table scans

## 11. The starter code provides the merge iterator interface to store `Box<I>` instead of `I`. What might be the reason behind that?

**Reasons for `Box<I>`**:

1. **Different iterator types**: Even though all implement `StorageIterator`, they might be:
   - `MemTableIterator`
   - `SstIterator` (later in the course)
   - `MergeIterator<MergeIterator<...>>` (nested)
   
   Without `Box`, the `MergeIterator` would need:
   ```rust
   struct MergeIterator<I1, I2, I3, ...>  // Impossible with Vec!
   ```

2. **Trait objects**: `Box<dyn StorageIterator>` allows storing heterogeneous iterators:
   ```rust
   Vec<Box<dyn StorageIterator>>  // Can mix different types
   ```

3. **Heap allocation**: Iterators might be large. Storing them on the heap:
   - Keeps `HeapWrapper` size small
   - Reduces memory movement during heap operations
   - Prevents stack overflow with many nested iterators

4. **Indirection**: Allows recursive/self-referential structures:
   ```rust
   MergeIterator<MergeIterator<MemTableIterator>>  // Would be infinite size!
   MergeIterator<Box<dyn StorageIterator>>         // Fixed size ✓
   ```

**Trade-off**: Extra indirection (pointer dereference) vs. flexibility and type erasure.