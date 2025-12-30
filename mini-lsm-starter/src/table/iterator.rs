// Copyright (c) 2022-2025 Alex Chi Z
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

use std::sync::Arc;

use anyhow::{Ok, Result};

use super::SsTable;
use crate::{block::BlockIterator, iterators::StorageIterator, key::KeySlice};

/// An iterator over the contents of an SSTable.
pub struct SsTableIterator {
    table: Arc<SsTable>,
    blk_iter: BlockIterator,
    blk_idx: usize,
}

impl SsTableIterator {
    /// Create a new iterator and seek to the first key-value pair in the first data block.
    pub fn create_and_seek_to_first(table: Arc<SsTable>) -> Result<Self> {
        let block = table.read_block_cached(0)?;
        let blk_iter = BlockIterator::create_and_seek_to_first(block);
        Ok(Self {
            table,
            blk_iter,
            blk_idx: 0,
        })
    }

    /// Seek to the first key-value pair in the first data block.
    pub fn seek_to_first(&mut self) -> Result<()> {
        let block = self.table.read_block_cached(0)?;
        let blk_iter = BlockIterator::create_and_seek_to_first(block);
        self.blk_iter = blk_iter;
        self.blk_idx = 0;
        Ok(())
    }

    /// Create a new iterator and seek to the first key-value pair which >= `key`.
    pub fn create_and_seek_to_key(table: Arc<SsTable>, key: KeySlice) -> Result<Self> {
        let mut iter = Self::create_and_seek_to_first(table)?;
        iter.seek_to_key(key)?;
        Ok(iter)
    }

    /// Seek to the first key-value pair which >= `key`.
    /// Note: You probably want to review the handout for detailed explanation when implementing
    /// this function.
    pub fn seek_to_key(&mut self, key: KeySlice) -> Result<()> {
        let metas = &self.table.block_meta;

        if metas.is_empty() {
            self.blk_idx = 0;
            // No need to issue a read here
            return Ok(());
        }

        // 1. Pick a block
        let mut blk_idx = self.table.find_block_idx(key);

        // 2. Seek in that block
        let mut blk_iter =
            BlockIterator::create_and_seek_to_key(self.table.read_block_cached(blk_idx)?, key);

        // Block with `blk_idx` is the best **candidate** that **might** contain a key >= target(key in para here) .
        // 3. Block has no key >= target, go to next
        if !blk_iter.is_valid() {
            blk_idx += 1;
            if blk_idx < self.table.num_of_blocks() {
                let next_block = self.table.read_block_cached(blk_idx)?;
                blk_iter = BlockIterator::create_and_seek_to_first(next_block);
            }
        }

        self.blk_iter = blk_iter;
        self.blk_idx = blk_idx;
        Ok(())
    }
}

impl StorageIterator for SsTableIterator {
    type KeyType<'a> = KeySlice<'a>;

    /// Return the `key` that's held by the underlying block iterator.
    fn key(&self) -> KeySlice<'_> {
        if !self.blk_iter.is_valid() {
            panic!("invalid sst iterator")
        }

        self.blk_iter.key()
    }

    /// Return the `value` that's held by the underlying block iterator.
    fn value(&self) -> &[u8] {
        if !self.blk_iter.is_valid() {
            panic!("invalid sst iterator")
        }

        self.blk_iter.value()
    }

    /// Return whether the current block iterator is valid or not.
    fn is_valid(&self) -> bool {
        self.blk_iter.is_valid()
    }

    /// Move to the next `key` in the block.
    /// Note: You may want to check if the current block iterator is valid after the move.
    fn next(&mut self) -> Result<()> {
        // Move current iter
        if !self.blk_iter.is_valid() {
            panic!("invalid block iterator")
        }

        self.blk_iter.next();

        // If it's still invalid, it means we should go to next block
        if !self.blk_iter.is_valid() {
            self.blk_idx += 1;
            if self.blk_idx < self.table.num_of_blocks() {
                let next_block = self.table.read_block_cached(self.blk_idx)?;
                self.blk_iter = BlockIterator::create_and_seek_to_first(next_block);
            }
        }

        Ok(())
    }
}
