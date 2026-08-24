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

use bytes::BufMut;

use crate::key::{KeySlice, KeyVec};

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

    /// Encoded size of the block if it were built right now:
    /// `data` section + 2 bytes per offset + 2 bytes for `num_of_elements`.
    fn estimated_size(&self) -> usize {
        self.data.len() + self.offsets.len() * 2 + 2
    }

    /// Adds a key-value pair to the block. Returns false when the block is full.
    /// You may find the `bytes::BufMut` trait useful for manipulating binary data.
    #[must_use]
    pub fn add(&mut self, key: KeySlice, value: &[u8]) -> bool {
        debug_assert!(!key.is_empty(), "key must not be empty");
        let key_len = key.len();
        let value_len = value.len();
        debug_assert!(key_len <= u16::MAX as usize, "key length must fit a u16");
        debug_assert!(
            value_len <= u16::MAX as usize,
            "value length must fit a u16"
        );

        // 6 = key_len(2) + value_len(2) + one offset(2); the num_of_elements
        // field is a fixed 2 bytes and does not grow per entry.
        let entry_size = 6 + key_len + value_len;
        // Reject only when the block is non-empty and the projected encoded
        // size would exceed the target. The first entry is always accepted so
        // the builder makes forward progress even on an oversized key.
        if !self.is_empty() && self.estimated_size() + entry_size > self.block_size {
            return false;
        }

        if self.is_empty() {
            self.first_key = key.to_key_vec();
        }
        self.offsets.push(self.data.len() as u16);
        self.data.put_u16_le(key_len as u16);
        self.data.put_slice(key.raw_ref());
        self.data.put_u16_le(value_len as u16);
        self.data.put_slice(value);
        true
    }

    /// Check if there is no key-value pair in the block.
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Finalize the block.
    pub fn build(self) -> Block {
        debug_assert!(!self.is_empty(), "block must not be empty");
        Block {
            data: self.data,
            offsets: self.offsets,
        }
    }
}
