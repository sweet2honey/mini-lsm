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

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use bytes::{BufMut, Bytes};

use super::{BlockMeta, FileObject, SsTable, bloom::Bloom};
use crate::{
    block::BlockBuilder,
    key::{KeyBytes, KeySlice},
    lsm_storage::BlockCache,
};

/// Builds an SSTable from key-value pairs.
pub struct SsTableBuilder {
    builder: BlockBuilder,
    first_key: Vec<u8>,
    last_key: Vec<u8>,
    data: Vec<u8>,
    pub(crate) meta: Vec<BlockMeta>,
    block_size: usize,
    /// One 32-bit hash per key added, in add order — input to the bloom filter.
    key_hashes: Vec<u32>,
}

impl SsTableBuilder {
    /// Create a builder based on target block size.
    pub fn new(block_size: usize) -> Self {
        Self {
            builder: BlockBuilder::new(block_size),
            first_key: Vec::new(),
            last_key: Vec::new(),
            data: Vec::new(),
            meta: Vec::new(),
            block_size,
            key_hashes: Vec::new(),
        }
    }

    /// Cut the current block: encode it into `data` and record its meta.
    ///
    /// The meta's `offset` is the length of `data` BEFORE appending — exactly
    /// where this block's bytes land in the final file.
    fn finish_block(&mut self) {
        debug_assert!(!self.builder.is_empty(), "finishing an empty block");
        let builder = std::mem::replace(&mut self.builder, BlockBuilder::new(self.block_size));
        let block = builder.build();
        self.meta.push(BlockMeta {
            offset: self.data.len(),
            first_key: KeyBytes::from_bytes(Bytes::copy_from_slice(&self.first_key)),
            last_key: KeyBytes::from_bytes(Bytes::copy_from_slice(&self.last_key)),
        });
        self.data.extend_from_slice(&block.encode());
        self.first_key.clear();
    }

    /// Adds a key-value pair to SSTable.
    ///
    /// Note: You should split a new block when the current block is full.(`std::mem::replace` may
    /// be helpful here)
    pub fn add(&mut self, key: KeySlice, value: &[u8]) {
        assert!(
            self.first_key.is_empty() || self.last_key.as_slice() <= key.raw_ref(),
            "keys must be added in sorted order"
        );
        // Hash now so build can construct the bloom filter with no second pass.
        self.key_hashes.push(farmhash::fingerprint32(key.raw_ref()));
        if self.builder.add(key, value) {
            if self.first_key.is_empty() {
                self.first_key.extend_from_slice(key.raw_ref());
            }
        } else {
            // Current block is full. Cut it, then place the key in a fresh
            // block — always succeeds (fresh-block first-entry carve-out).
            self.finish_block();
            assert!(self.builder.add(key, value));
            self.first_key.extend_from_slice(key.raw_ref());
        }
        self.last_key.clear();
        self.last_key.extend_from_slice(key.raw_ref());
    }

    /// Get the estimated size of the SSTable.
    ///
    /// Since the data blocks contain much more data than meta blocks, just return the size of data
    /// blocks here.
    pub fn estimated_size(&self) -> usize {
        self.data.len()
    }

    /// Builds the SSTable and writes it to the given path. Use the `FileObject` structure to
    /// manipulate the disk objects.
    pub fn build(
        mut self,
        id: usize,
        block_cache: Option<Arc<BlockCache>>,
        path: impl AsRef<Path>,
    ) -> Result<SsTable> {
        self.finish_block();
        let mut buf = self.data;
        // Footer (Day 7 book layout): metadata | meta_offset | bloom | bloom_offset.
        // Each offset is captured when buf reaches its section's start, appended
        // after the section ends.
        let meta_offset = buf.len();
        BlockMeta::encode_block_meta(&self.meta, &mut buf);
        buf.put_u32_le(meta_offset as u32);
        let bloom_offset = buf.len();
        let bloom = Bloom::build_from_key_hashes(
            &self.key_hashes,
            Bloom::bloom_bits_per_key(self.key_hashes.len(), 0.01),
        );
        bloom.encode(&mut buf);
        buf.put_u32_le(bloom_offset as u32);
        let file = FileObject::create(path.as_ref(), buf)?;
        Ok(SsTable {
            file,
            block_meta_offset: meta_offset,
            id,
            block_cache,
            first_key: self.meta.first().unwrap().first_key.clone(),
            last_key: self.meta.last().unwrap().last_key.clone(),
            bloom: Some(bloom),
            max_ts: 0,
            block_meta: self.meta,
        })
    }

    #[cfg(test)]
    pub(crate) fn build_for_test(self, path: impl AsRef<Path>) -> Result<SsTable> {
        self.build(0, None, path)
    }
}
