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

use std::{cmp::Ordering, sync::Arc};

use bytes::Buf;

use crate::key::{Key, KeySlice, KeyVec};

use super::Block;

/// Iterates on a block.
pub struct BlockIterator {
    /// The internal `Block`, wrapped by an `Arc`
    block: Arc<Block>,
    /// The current key, empty represents the iterator is invalid
    key: KeyVec,
    /// the current value range in the block.data, corresponds to the current key
    value_range: (usize, usize),
    /// Current index of the key-value pair, should be in range of [0, num_of_elements)
    idx: usize,
    /// The first key in the block
    first_key: KeyVec,
}

impl BlockIterator {
    fn new(block: Arc<Block>) -> Self {
        Self {
            block,
            key: KeyVec::new(),
            value_range: (0, 0),
            idx: 0,
            first_key: KeyVec::new(),
        }
    }

    /// Creates a block iterator and seek to the first entry.
    pub fn create_and_seek_to_first(block: Arc<Block>) -> Self {
        let mut iter = Self::new(block);
        iter.seek_to_first();
        iter
    }

    /// Creates a block iterator and seek to the first key that >= `key`.
    pub fn create_and_seek_to_key(block: Arc<Block>, key: KeySlice) -> Self {
        let mut iter = Self::new(block);
        iter.seek_to_key(key);
        iter
    }

    fn len(&self) -> usize {
        self.block.offsets.len()
    }

    /// Returns the key of the current entry.
    pub fn key(&self) -> KeySlice<'_> {
        self.key.as_key_slice()
    }

    /// Returns the value of the current entry.
    pub fn value(&self) -> &[u8] {
        &self.block.data[self.value_range.0..self.value_range.1]
    }

    /// Returns true if the iterator is valid.
    /// Note: You may want to make use of `key`
    pub fn is_valid(&self) -> bool {
        !self.key.is_empty()
    }

    /// Seeks to the first key in the block.
    pub fn seek_to_first(&mut self) {
        self.seek_to_index(0);
        self.first_key = self.key.clone();
    }

    /// Move to the next key in the block.
    pub fn next(&mut self) {
        self.seek_to_index(self.idx + 1);
    }

    /// Seek to the first key that >= `key`.
    /// Note: You should assume the key-value pairs in the block are sorted when being added by
    /// callers.
    pub fn seek_to_key(&mut self, key: KeySlice) {
        //* Under the assumption of sorted-keys in blocks, it is optimal to do binary search over `block.offset`
        // Perform a open-range [low, high) binary search
        let mut low = 0;
        let mut high = self.len();

        while low < high {
            let mid = low + (high - low) / 2;

            self.seek_to_index(mid);

            match self.key().cmp(&key) {
                Ordering::Less => low = mid + 1, // search right
                Ordering::Greater => high = mid, // search left
                Ordering::Equal => return,
            }
        }

        // low is the first index where key >= target
        self.seek_to_index(low);
    }

    /// Single source of truth: updates idx, key, and value_range together.
    /// Handles invalidation when idx is out of bounds.
    fn seek_to_index(&mut self, idx: usize) {
        if idx >= self.len() {
            // Invalidate
            self.idx = idx;
            self.key = KeyVec::new();
            self.value_range = (0, 0);
            return;
        }

        self.idx = idx;

        // `block.offset` points to entrys' bytes data, where you can decode the key out
        let entry_start = self.block.offsets[idx] as usize;

        // Decode key
        let mut buf = &self.block.data[entry_start..];
        let key_len = buf.get_u16() as usize;
        self.key.set_from_slice(Key::from_slice(&buf[..key_len]));

        // Locate value
        buf.advance(key_len); // Skip key
        let value_len = buf.get_u16() as usize;
        let value_start = entry_start + 2 + key_len + 2; // Skip value len as well
        self.value_range = (value_start, value_start + value_len);
    }
}
