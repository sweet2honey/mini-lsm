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

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use bytes::{BufMut, Bytes};

use super::{BlockMeta, SsTable};
use crate::{
    block::{self, Block, BlockBuilder},
    key::{Key, KeyBytes, KeySlice},
    lsm_storage::BlockCache,
    table::FileObject,
};

/// Builds an SSTable from key-value pairs.
pub struct SsTableBuilder {
    builder: BlockBuilder,
    first_key: Vec<u8>, // For current block
    last_key: Vec<u8>,  // For current block
    data: Vec<u8>,
    pub(crate) meta: Vec<BlockMeta>,
    block_size: usize,
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
        }
    }

    /// Adds a key-value pair to SSTable.
    ///
    /// Note: You should split a new block when the current block is full.(`std::mem::replace` may
    /// be helpful here)
    pub fn add(&mut self, key: KeySlice, value: &[u8]) {
        // Update `first_key` as needed
        if self.first_key.is_empty() {
            self.first_key.clear();
            self.first_key.extend(key.raw_ref());
        }

        //* If current block is full, finalize it and create a new one
        if !self.builder.add(key, value) {
            self.finalize_block();
            // Should success now
            assert!(self.builder.add(key, value));
            self.first_key.clear();
            self.first_key.extend(key.raw_ref());
            self.last_key.clear();
            self.last_key.extend(key.raw_ref());
        } else {
            // Just update `last_key` on success path
            self.last_key.clear();
            self.last_key.extend(key.raw_ref());
        }
    }

    /// Get the estimated size of the SSTable.
    ///
    /// Since the data blocks contain much more data than meta blocks, just return the size of data
    /// blocks here.
    pub fn estimated_size(&self) -> usize {
        self.data.len()
    }

    /// Builds the SSTable and writes it to the given path. Use the `FileObject` structure to manipulate the disk objects.
    pub fn build(
        mut self,
        id: usize,
        block_cache: Option<Arc<BlockCache>>,
        path: impl AsRef<Path>,
    ) -> Result<SsTable> {
        // Finalize in-use block
        self.finalize_block();

        //* A SST is made up of:
        // 1. [Block, Block, ...,
        // Now all block data is in `self.data`
        let mut data = self.data;

        // 2. Meta,
        let block_meta_offset = data.len();
        BlockMeta::encode_block_meta(&self.meta, &mut data);

        // 3. Meta Offset]
        data.put_u32(block_meta_offset as u32);

        // Dump to file
        let file = FileObject::create(path.as_ref(), data)?;

        Ok(SsTable {
            file,
            block_meta_offset,
            id,
            block_cache,
            //* These are the SST's keys, should read from metadata
            // Since we have called `finalize_block`, there should be at least one block meta
            first_key: self.meta.first().unwrap().first_key.clone(),
            last_key: self.meta.last().unwrap().last_key.clone(),
            block_meta: self.meta, // use after access above
            bloom: None,
            max_ts: 0,
        })
    }

    /// Finalize the current block info:
    /// 1. Encode current block, store to `self.data`
    /// 2. Update block metas
    /// 3. Reset block-related members
    fn finalize_block(&mut self) {
        // Build BlockMeta, should be done before append block data to get the correct offset
        let block_meta = BlockMeta {
            offset: self.data.len(),
            first_key: Key::from_bytes(Bytes::copy_from_slice(&self.first_key)),
            last_key: Key::from_bytes(Bytes::copy_from_slice(&self.last_key)),
        };
        self.meta.push(block_meta);

        // Add block data
        let builder = std::mem::replace(&mut self.builder, BlockBuilder::new(self.block_size));
        let block_bytes = builder.build().encode();
        self.data.append(&mut block_bytes.to_vec());
    }

    #[cfg(test)]
    pub(crate) fn build_for_test(self, path: impl AsRef<Path>) -> Result<SsTable> {
        self.build(0, None, path)
    }
}
