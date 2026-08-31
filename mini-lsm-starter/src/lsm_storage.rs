// Copyright (c) 2022-2026 Alex Chi Z
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![allow(unused_variables)] // TODO(you): remove this lint after implementing this mod
#![allow(dead_code)] // TODO(you): remove this lint after implementing this mod

use std::collections::HashMap;
use std::ops::Bound;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;

use anyhow::Result;
use bytes::Bytes;
use parking_lot::{Mutex, MutexGuard, RwLock};

use crate::block::Block;
use crate::compact::{
    CompactionController, CompactionOptions, LeveledCompactionController, LeveledCompactionOptions,
    SimpleLeveledCompactionController, SimpleLeveledCompactionOptions, TieredCompactionController,
};
use crate::iterators::StorageIterator;
use crate::iterators::merge_iterator::MergeIterator;
use crate::iterators::two_merge_iterator::TwoMergeIterator;
use crate::key::KeySlice;
use crate::lsm_iterator::{FusedIterator, LsmIterator};
use crate::manifest::Manifest;
use crate::mem_table::{MemTable, MemTableIterator};
use crate::mvcc::LsmMvccInner;
use crate::table::{SsTable, SsTableBuilder, SsTableIterator};

pub type BlockCache = moka::sync::Cache<(usize, usize), Arc<Block>>;

/// Represents the state of the storage engine.
#[derive(Clone)]
pub struct LsmStorageState {
    /// The current memtable.
    pub memtable: Arc<MemTable>,
    /// Immutable memtables, from latest to earliest.
    pub imm_memtables: Vec<Arc<MemTable>>,
    /// L0 SSTs, from latest to earliest.
    pub l0_sstables: Vec<usize>,
    /// SsTables sorted by key range; L1 - L_max for leveled compaction, or tiers for tiered
    /// compaction.
    pub levels: Vec<(usize, Vec<usize>)>,
    /// SST objects.
    pub sstables: HashMap<usize, Arc<SsTable>>,
}

pub enum WriteBatchRecord<T: AsRef<[u8]>> {
    Put(T, T),
    Del(T),
}

impl LsmStorageState {
    fn create(options: &LsmStorageOptions) -> Self {
        let levels = match &options.compaction_options {
            CompactionOptions::Leveled(LeveledCompactionOptions { max_levels, .. })
            | CompactionOptions::Simple(SimpleLeveledCompactionOptions { max_levels, .. }) => (1
                ..=*max_levels)
                .map(|level| (level, Vec::new()))
                .collect::<Vec<_>>(),
            CompactionOptions::Tiered(_) => Vec::new(),
            CompactionOptions::NoCompaction => vec![(1, Vec::new())],
        };
        Self {
            memtable: Arc::new(MemTable::create(0)),
            imm_memtables: Vec::new(),
            l0_sstables: Vec::new(),
            levels,
            sstables: Default::default(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LsmStorageOptions {
    // Block size in bytes
    pub block_size: usize,
    // SST size in bytes, also the approximate memtable capacity limit
    pub target_sst_size: usize,
    // Maximum number of memtables in memory, flush to L0 when exceeding this limit
    pub num_memtable_limit: usize,
    pub compaction_options: CompactionOptions,
    pub enable_wal: bool,
    pub serializable: bool,
}

impl LsmStorageOptions {
    pub fn default_for_week1_test() -> Self {
        Self {
            block_size: 4096,
            target_sst_size: 2 << 20,
            compaction_options: CompactionOptions::NoCompaction,
            enable_wal: false,
            num_memtable_limit: 50,
            serializable: false,
        }
    }

    pub fn default_for_week1_day6_test() -> Self {
        Self {
            block_size: 4096,
            target_sst_size: 2 << 20,
            compaction_options: CompactionOptions::NoCompaction,
            enable_wal: false,
            num_memtable_limit: 2,
            serializable: false,
        }
    }

    pub fn default_for_week2_test(compaction_options: CompactionOptions) -> Self {
        Self {
            block_size: 4096,
            target_sst_size: 1 << 20, // 1MB
            compaction_options,
            enable_wal: false,
            num_memtable_limit: 2,
            serializable: false,
        }
    }
}

#[derive(Clone, Debug)]
pub enum CompactionFilter {
    Prefix(Bytes),
}

/// The storage interface of the LSM tree.
pub(crate) struct LsmStorageInner {
    pub(crate) state: Arc<RwLock<Arc<LsmStorageState>>>,
    pub(crate) state_lock: Mutex<()>,
    path: PathBuf,
    pub(crate) block_cache: Arc<BlockCache>,
    next_sst_id: AtomicUsize,
    pub(crate) options: Arc<LsmStorageOptions>,
    pub(crate) compaction_controller: CompactionController,
    pub(crate) manifest: Option<Manifest>,
    pub(crate) mvcc: Option<LsmMvccInner>,
    pub(crate) compaction_filters: Arc<Mutex<Vec<CompactionFilter>>>,
}

/// A thin wrapper for `LsmStorageInner` and the user interface for MiniLSM.
pub struct MiniLsm {
    pub(crate) inner: Arc<LsmStorageInner>,
    /// Notifies the L0 flush thread to stop working. (In week 1 day 6)
    flush_notifier: crossbeam_channel::Sender<()>,
    /// The handle for the flush thread. (In week 1 day 6)
    flush_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Notifies the compaction thread to stop working. (In week 2)
    compaction_notifier: crossbeam_channel::Sender<()>,
    /// The handle for the compaction thread. (In week 2)
    compaction_thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl Drop for MiniLsm {
    fn drop(&mut self) {
        self.compaction_notifier.send(()).ok();
        self.flush_notifier.send(()).ok();
    }
}

impl MiniLsm {
    pub fn close(&self) -> Result<()> {
        // Signal the background threads to stop, then wait for them, so no flush or
        // compaction work races with the shutdown drain below.
        self.compaction_notifier.send(()).ok();
        self.flush_notifier.send(()).ok();
        if let Some(handle) = self.compaction_thread.lock().take() {
            handle
                .join()
                .map_err(|e| anyhow::anyhow!("compaction thread panicked: {e:?}"))?;
        }
        if let Some(handle) = self.flush_thread.lock().take() {
            handle
                .join()
                .map_err(|e| anyhow::anyhow!("flush thread panicked: {e:?}"))?;
        }
        // Drain any immutable memtables still in memory so every frozen table reaches L0.
        while !self.inner.state.read().imm_memtables.is_empty() {
            self.inner.force_flush_next_imm_memtable()?;
        }
        Ok(())
    }

    /// Start the storage engine by either loading an existing directory or creating a new one if the directory does
    /// not exist.
    pub fn open(path: impl AsRef<Path>, options: LsmStorageOptions) -> Result<Arc<Self>> {
        let inner = Arc::new(LsmStorageInner::open(path, options)?);
        let (tx1, rx) = crossbeam_channel::unbounded();
        let compaction_thread = inner.spawn_compaction_thread(rx)?;
        let (tx2, rx) = crossbeam_channel::unbounded();
        let flush_thread = inner.spawn_flush_thread(rx)?;
        Ok(Arc::new(Self {
            inner,
            flush_notifier: tx2,
            flush_thread: Mutex::new(flush_thread),
            compaction_notifier: tx1,
            compaction_thread: Mutex::new(compaction_thread),
        }))
    }

    pub fn new_txn(&self) -> Result<()> {
        self.inner.new_txn()
    }

    pub fn write_batch<T: AsRef<[u8]>>(&self, batch: &[WriteBatchRecord<T>]) -> Result<()> {
        self.inner.write_batch(batch)
    }

    pub fn add_compaction_filter(&self, compaction_filter: CompactionFilter) {
        self.inner.add_compaction_filter(compaction_filter)
    }

    pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        self.inner.get(key)
    }

    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        self.inner.put(key, value)
    }

    pub fn delete(&self, key: &[u8]) -> Result<()> {
        self.inner.delete(key)
    }

    pub fn sync(&self) -> Result<()> {
        self.inner.sync()
    }

    pub fn scan(
        &self,
        lower: Bound<&[u8]>,
        upper: Bound<&[u8]>,
    ) -> Result<FusedIterator<LsmIterator>> {
        self.inner.scan(lower, upper)
    }

    /// Only call this in test cases due to race conditions
    pub fn force_flush(&self) -> Result<()> {
        if !self.inner.state.read().memtable.is_empty() {
            self.inner
                .force_freeze_memtable(&self.inner.state_lock.lock())?;
        }
        if !self.inner.state.read().imm_memtables.is_empty() {
            self.inner.force_flush_next_imm_memtable()?;
        }
        Ok(())
    }

    pub fn force_full_compaction(&self) -> Result<()> {
        self.inner.force_full_compaction()
    }
}

/// Whether an SST whose key range is `[first, last]` can contribute any key to a scan
/// over (lower, upper). When unsure, keep the SST: over-keeping costs I/O, while
/// over-skipping silently loses keys.
fn range_overlap(lower: Bound<&[u8]>, upper: Bound<&[u8]>, first: &[u8], last: &[u8]) -> bool {
    let above_last = match lower {
        Bound::Included(k) => k > last,
        Bound::Excluded(k) => k >= last,
        Bound::Unbounded => false,
    };
    let below_first = match upper {
        Bound::Included(k) => k < first,
        Bound::Excluded(k) => k <= first,
        Bound::Unbounded => false,
    };
    !above_last && !below_first
}

/// Whether `key` may exist in an SST spanning `[first, last]`: a point query is the
/// closed-edge overlap degenerated to `first <= key <= last`.
fn key_within(key: &[u8], first: &[u8], last: &[u8]) -> bool {
    first <= key && key <= last
}

impl LsmStorageInner {
    pub(crate) fn next_sst_id(&self) -> usize {
        self.next_sst_id
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn mvcc(&self) -> &LsmMvccInner {
        self.mvcc.as_ref().unwrap()
    }

    /// Start the storage engine by either loading an existing directory or creating a new one if the directory does
    /// not exist.
    pub(crate) fn open(path: impl AsRef<Path>, options: LsmStorageOptions) -> Result<Self> {
        let path = path.as_ref();
        // The database directory may not exist yet; the first flush would fail to
        // create its SST file without it.
        std::fs::create_dir_all(path)?;
        let state = LsmStorageState::create(&options);

        let compaction_controller = match &options.compaction_options {
            CompactionOptions::Leveled(options) => {
                CompactionController::Leveled(LeveledCompactionController::new(options.clone()))
            }
            CompactionOptions::Tiered(options) => {
                CompactionController::Tiered(TieredCompactionController::new(options.clone()))
            }
            CompactionOptions::Simple(options) => CompactionController::Simple(
                SimpleLeveledCompactionController::new(options.clone()),
            ),
            CompactionOptions::NoCompaction => CompactionController::NoCompaction,
        };

        let storage = Self {
            state: Arc::new(RwLock::new(Arc::new(state))),
            state_lock: Mutex::new(()),
            path: path.to_path_buf(),
            block_cache: Arc::new(BlockCache::new(1024)),
            next_sst_id: AtomicUsize::new(1),
            compaction_controller,
            manifest: None,
            options: options.into(),
            mvcc: None,
            compaction_filters: Arc::new(Mutex::new(Vec::new())),
        };

        Ok(storage)
    }

    pub fn sync(&self) -> Result<()> {
        unimplemented!()
    }

    pub fn add_compaction_filter(&self, compaction_filter: CompactionFilter) {
        let mut compaction_filters = self.compaction_filters.lock();
        compaction_filters.push(compaction_filter);
    }

    /// Get a key from the storage. In day 7, this can be further optimized by using a bloom filter.
    pub fn get(&self, key: &[u8]) -> Result<Option<Bytes>> {
        let snapshot = self.state.read().clone();

        // Probe the mutable memtable first, then immutable memtables newest-to-oldest.
        // The first memtable that holds the key decides: an empty value is a tombstone
        // (key is deleted), a non-empty value is the answer, and no hit means absent.
        for memtable in std::iter::once(&snapshot.memtable).chain(snapshot.imm_memtables.iter()) {
            match memtable.get(key) {
                Some(value) if value.is_empty() => return Ok(None),
                Some(value) => return Ok(Some(value)),
                None => continue,
            }
        }

        // No memtable holds the key: fall through to L0 SSTs. The read guard was
        // already dropped when `snapshot` was cloned, so seeking here does its I/O
        // off-lock. l0_sstables is newest->oldest, so the merge's index tie-break
        // keeps precedence.
        let mut sst_iters: Vec<Box<SsTableIterator>> =
            Vec::with_capacity(snapshot.l0_sstables.len());
        for sst_id in snapshot.l0_sstables.iter() {
            let sst = snapshot
                .sstables
                .get(sst_id)
                .expect("l0_sstables references a missing SST")
                .clone();
            // Optimization only: an SST whose range cannot hold `key` is skipped
            // before spending a block read on its seek.
            if !key_within(key, sst.first_key().raw_ref(), sst.last_key().raw_ref()) {
                continue;
            }
            sst_iters.push(Box::new(SsTableIterator::create_and_seek_to_key(
                sst,
                KeySlice::from_slice(key),
            )?));
        }
        let iter = MergeIterator::create(sst_iters);
        // A lower-bound seek may land on the next greater key, so a value counts only
        // on exact key equality (invariant 4). An empty value is a tombstone: the key
        // is deleted, not "absent, keep looking" — the merge already resolved any
        // duplicate, so the head entry is final.
        if !iter.is_valid() || iter.key().raw_ref() != key {
            return Ok(None);
        }
        let value = iter.value();
        if value.is_empty() {
            return Ok(None);
        }
        Ok(Some(Bytes::copy_from_slice(value)))
    }

    /// Write a batch of data into the storage. Implement in week 2 day 7.
    pub fn write_batch<T: AsRef<[u8]>>(&self, _batch: &[WriteBatchRecord<T>]) -> Result<()> {
        unimplemented!()
    }

    /// Put a key-value pair into the storage by writing into the current memtable.
    pub fn put(&self, key: &[u8], value: &[u8]) -> Result<()> {
        let memtable = self.state.read().memtable.clone();
        memtable.put(key, value)?;

        if memtable.approximate_size() > self.options.target_sst_size {
            let state_lock = self.state_lock.lock();
            if self.state.read().memtable.approximate_size() > self.options.target_sst_size {
                self.force_freeze_memtable(&state_lock)?;
            }
        }
        Ok(())
    }

    /// Remove a key from the storage by writing an empty value.
    pub fn delete(&self, key: &[u8]) -> Result<()> {
        self.put(key, b"")
    }

    pub(crate) fn path_of_sst_static(path: impl AsRef<Path>, id: usize) -> PathBuf {
        path.as_ref().join(format!("{:05}.sst", id))
    }

    pub(crate) fn path_of_sst(&self, id: usize) -> PathBuf {
        Self::path_of_sst_static(&self.path, id)
    }

    pub(crate) fn path_of_wal_static(path: impl AsRef<Path>, id: usize) -> PathBuf {
        path.as_ref().join(format!("{:05}.wal", id))
    }

    pub(crate) fn path_of_wal(&self, id: usize) -> PathBuf {
        Self::path_of_wal_static(&self.path, id)
    }

    pub(super) fn sync_dir(&self) -> Result<()> {
        unimplemented!()
    }

    /// Force freeze the current memtable to an immutable memtable
    pub fn force_freeze_memtable(&self, _state_lock_observer: &MutexGuard<'_, ()>) -> Result<()> {
        let new_memtable = Arc::new(MemTable::create(self.next_sst_id()));
        let mut guard = self.state.write();
        let mut snapshot = guard.as_ref().clone();
        let old_memtable = std::mem::replace(&mut snapshot.memtable, new_memtable);
        snapshot.imm_memtables.insert(0, old_memtable);
        *guard = Arc::new(snapshot);
        Ok(())
    }

    /// Force flush the earliest-created immutable memtable to disk
    pub fn force_flush_next_imm_memtable(&self) -> Result<()> {
        let _state_lock = self.state_lock.lock();

        // Select the oldest immutable memtable (the last in the list). Selection and
        // installation both hold state_lock, so two flushes can never choose or remove
        // the same memtable.
        let memtable_to_flush = {
            let guard = self.state.read();
            let Some(memtable) = guard.imm_memtables.last() else {
                return Ok(());
            };
            memtable.clone()
        };

        // Build the SST holding no state lock at all: the source memtable is frozen,
        // so its contents cannot change, and block encoding + file I/O is far too slow
        // to run under a lock that readers and writers share.
        let sst_id = memtable_to_flush.id();
        let mut builder = SsTableBuilder::new(self.options.block_size);
        memtable_to_flush.flush(&mut builder)?;
        let sst = builder.build(
            sst_id,
            Some(self.block_cache.clone()),
            self.path_of_sst(sst_id),
        )?;

        // Install atomically: remove exactly the memtable that was flushed and register
        // the SST (newest side of L0) in a single snapshot swap. Readers see either the
        // full old state or the full new state.
        {
            let mut guard = self.state.write();
            let mut snapshot = guard.as_ref().clone();
            let removed = snapshot
                .imm_memtables
                .pop()
                .expect("immutable memtable vanished between selection and install; only flushes remove it, and state_lock serializes flushes");
            assert_eq!(
                removed.id(),
                sst_id,
                "flushed SST must correspond to the popped immutable memtable"
            );
            snapshot.l0_sstables.insert(0, sst_id);
            snapshot.sstables.insert(sst_id, Arc::new(sst));
            *guard = Arc::new(snapshot);
        }
        Ok(())
    }

    pub fn new_txn(&self) -> Result<()> {
        // no-op
        Ok(())
    }

    /// Create an iterator over a range of keys.
    pub fn scan(
        &self,
        lower: Bound<&[u8]>,
        upper: Bound<&[u8]>,
    ) -> Result<FusedIterator<LsmIterator>> {
        // Clone the Arc<LsmStorageState> and drop the read guard before building any
        // SST iterator: creating/seeking an SsTableIterator may read a block from disk.
        let snapshot = self.state.read().clone();

        // --- memtable merge: SkipMap::range enforces both bounds natively ---
        let mut memtable_iters: Vec<Box<MemTableIterator>> =
            Vec::with_capacity(snapshot.imm_memtables.len() + 1);
        memtable_iters.push(Box::new(snapshot.memtable.scan(lower, upper)));
        for memtable in snapshot.imm_memtables.iter() {
            memtable_iters.push(Box::new(memtable.scan(lower, upper)));
        }
        let memtable_merge = MergeIterator::create(memtable_iters);

        // --- L0 SST merge: l0_sstables is newest->oldest, so MergeIterator's index
        // tie-break keeps the newer SST on top. The lower bound is applied per-SST at
        // seek time (SsTableIterator has no end-bound seek); the upper bound is
        // enforced later in LsmIterator. ---
        let mut sst_iters: Vec<Box<SsTableIterator>> =
            Vec::with_capacity(snapshot.l0_sstables.len());
        for sst_id in snapshot.l0_sstables.iter() {
            let sst = snapshot
                .sstables
                .get(sst_id)
                .expect("l0_sstables references a missing SST")
                .clone();
            // Optimization only: skip SSTs whose [first,last] cannot overlap the
            // requested range; the result stream is unchanged (book invariant 5).
            if !range_overlap(
                lower,
                upper,
                sst.first_key().raw_ref(),
                sst.last_key().raw_ref(),
            ) {
                continue;
            }
            let iter = match &lower {
                Bound::Included(key) => {
                    SsTableIterator::create_and_seek_to_key(sst, KeySlice::from_slice(key))?
                }
                Bound::Excluded(key) => {
                    let mut it =
                        SsTableIterator::create_and_seek_to_key(sst, KeySlice::from_slice(key))?;
                    // Lower-bound seek lands on `key` itself when present; the bound
                    // excludes it, so skip that one entry. Intra-SST keys are unique,
                    // so a single advance suffices.
                    if it.is_valid() && it.key().raw_ref() == *key {
                        it.next()?;
                    }
                    it
                }
                Bound::Unbounded => SsTableIterator::create_and_seek_to_first(sst)?,
            };
            sst_iters.push(Box::new(iter));
        }
        let sst_merge = MergeIterator::create(sst_iters);

        // A = memtable merge (newer), B = SST merge (older): TwoMergeIterator prefers
        // A on ties, matching memtable-precedence-over-L0.
        let two_merge = TwoMergeIterator::create(memtable_merge, sst_merge)?;
        let lsm_iter = LsmIterator::new(two_merge, upper.map(Bytes::copy_from_slice))?;
        Ok(FusedIterator::new(lsm_iter))
    }
}
