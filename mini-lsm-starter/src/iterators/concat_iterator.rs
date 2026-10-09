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

use std::sync::Arc;

use anyhow::Result;

use super::StorageIterator;
use crate::{
    key::KeySlice,
    table::{SsTable, SsTableIterator},
};

/// Concat multiple iterators ordered in key order and their key ranges do not overlap. We do not want to create the
/// iterators when initializing this iterator to reduce the overhead of seeking.
pub struct SstConcatIterator {
    current: Option<SsTableIterator>,
    next_sst_idx: usize,
    sstables: Vec<Arc<SsTable>>,
}

impl SstConcatIterator {
    /// Precondition (caller discipline, enforced by construction in compaction):
    /// `sstables` are sorted by first key and their key ranges do not overlap.
    /// The seek performs NO overlap validation; using this iterator on an
    /// overlapping run silently skips or resurrects keys.
    pub fn create_and_seek_to_first(sstables: Vec<Arc<SsTable>>) -> Result<Self> {
        let mut iter = Self {
            current: None,
            next_sst_idx: 0,
            sstables,
        };
        // An empty run (e.g. L1 before the first compaction) is a valid
        // construction: the iterator is simply never valid.
        if !iter.sstables.is_empty() {
            iter.skip_to_sst(0)?;
        }
        Ok(iter)
    }

    /// Lower-bound seek across the run: opens the LAST SST whose first_key is
    /// <= `key` (same rule as Day-4 find_block_idx, one level up). If that
    /// in-SST seek lands invalid, every key in this SST is < `key` while the
    /// next SST's first_key is > `key` (non-overlap invariant), so its FIRST
    /// entry is exactly the run's lower bound — a single hop suffices, no loop.
    pub fn create_and_seek_to_key(sstables: Vec<Arc<SsTable>>, key: KeySlice) -> Result<Self> {
        if sstables.is_empty() {
            return Ok(Self {
                current: None,
                next_sst_idx: 0,
                sstables,
            });
        }
        let idx = sstables
            .partition_point(|sst| sst.first_key().raw_ref() <= key.raw_ref())
            .saturating_sub(1);
        let mut iter = Self {
            current: None,
            next_sst_idx: idx,
            sstables,
        };
        let sst = iter.sstables[idx].clone();
        let current = SsTableIterator::create_and_seek_to_key(sst, key)?;
        if !current.is_valid() {
            // All keys in SST idx are < key; the next SST's first entry is the
            // lower bound. idx is the last candidate, so never loop.
            iter.next_sst_idx += 1;
            iter.skip_to_sst(iter.next_sst_idx)?;
        } else {
            iter.current = Some(current);
        }
        Ok(iter)
    }

    /// Point `current` at the first entry of `sstables[idx]`. Exhausted runs
    /// leave `current = None` and `next_sst_idx = sstables.len()` (one past the
    /// end), which is_valid reads as invalid.
    fn skip_to_sst(&mut self, idx: usize) -> Result<()> {
        if idx < self.sstables.len() {
            // seek_to_first, never a stale search key: the landing rule is
            // self-contained (first entry) and mirrors Day-4 advance_if_needed.
            self.current = Some(SsTableIterator::create_and_seek_to_first(
                self.sstables[idx].clone(),
            )?);
        } else {
            self.current = None;
        }
        self.next_sst_idx = idx + 1;
        Ok(())
    }
}

impl StorageIterator for SstConcatIterator {
    type KeyType<'a> = KeySlice<'a>;

    fn key(&self) -> KeySlice<'_> {
        self.current
            .as_ref()
            .expect("key() called on exhausted SstConcatIterator")
            .key()
    }

    fn value(&self) -> &[u8] {
        self.current
            .as_ref()
            .expect("value() called on exhausted SstConcatIterator")
            .value()
    }

    fn is_valid(&self) -> bool {
        self.current.as_ref().is_some_and(|it| it.is_valid())
    }

    fn next(&mut self) -> Result<()> {
        if let Some(current) = self.current.as_mut() {
            current.next()?;
            if !current.is_valid() {
                self.skip_to_sst(self.next_sst_idx)?;
            }
        }
        Ok(())
    }

    // Inherited default: always 1 (book: "it should always report one active
    // iterator" — only the active child exists).
}
