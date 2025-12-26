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

use crate::key::{KeySlice, KeyVec};

use bytes::BufMut;

use super::Block;

/// Builds a block.
pub struct BlockBuilder {
    /// Offsets of each key-value entries.
    offsets: Vec<u16>,
    /// All serialized key-value pairs in the block.
    data: Vec<u8>,
    /// The expected block size.
    block_size: usize,
    /// The first key in the block
    first_key: KeyVec,
}

impl BlockBuilder {
    /// Creates a new block builder.
    pub fn new(block_size: usize) -> Self {
        Self {
            offsets: Vec::new(),
            data: Vec::new(),
            block_size,
            first_key: KeyVec::new(),
        }
    }

    /// Get the size of block if built
    fn size(&self) -> usize {
        // offsets + data + extra(num of elements)
        self.offsets.len() * 2 + self.data.len() + size_of::<u16>()
    }

    /// Adds a key-value pair to the block. Returns false when the block is full.
    /// You may find the `bytes::BufMut` trait useful for manipulating binary data.
    #[must_use]
    pub fn add(&mut self, key: KeySlice, value: &[u8]) -> bool {
        // Adding a key-value pair requires extra space to record the metadata
        const KEY_LEN: usize = 2;
        const VALUE_LEN: usize = 2;
        const OFFSET_LEN: usize = 2;
        const META_LEN: usize = KEY_LEN + VALUE_LEN + OFFSET_LEN;

        let pair_len = key.len() + value.len() + META_LEN;

        // Check block full or not
        //* Only fail on non-empty data, to make storing a big key-value pair possible
        //* tested in tests: test_block_build_large_1, test_block_build_large_2
        if self.size() + pair_len > self.block_size && !self.is_empty() {
            return false;
        }

        // Update first key
        if self.first_key.is_empty() {
            self.first_key.set_from_slice(key);
        }

        self.offsets.push(self.data.len() as u16); // Current len is new entry's start offset
        self.data.put_u16(key.len() as u16);
        self.data.put(key.raw_ref());
        self.data.put_u16(value.len() as u16);
        self.data.put(value);
        true
    }

    /// Check if there is no key-value pair in the block.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Finalize the block.
    pub fn build(self) -> Block {
        Block {
            data: self.data,
            offsets: self.offsets,
        }
    }
}
