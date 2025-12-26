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

mod builder;
mod iterator;

pub use builder::BlockBuilder;
use bytes::{Buf, BufMut, Bytes};
pub use iterator::BlockIterator;

/// A block is the smallest unit of read and caching in LSM tree. It is a collection of sorted key-value pairs.
pub struct Block {
    pub(crate) data: Vec<u8>,
    pub(crate) offsets: Vec<u16>,
}

impl Block {
    /// Encode the internal data to the data layout illustrated in the course
    /// Note: You may want to recheck if any of the expected field is missing from your output
    pub fn encode(&self) -> Bytes {
        let mut vec = self.data.clone();
        for offset in &self.offsets {
            vec.put_u16(*offset);
        }
        vec.put_u16(self.offsets.len() as u16);
        Bytes::from(vec)
    }

    /// Decode from the data layout, transform the input `data` to a single `Block`
    pub fn decode(data: &[u8]) -> Self {
        let len = data.len();

        if len < size_of::<u16>() {
            panic!("invalid data to decode");
        }

        // Read num of entrys at data tail
        let mut cursor = &data[len - 2..];
        let num_of_entry = cursor.get_u16() as usize;

        // With num of entrys, we know length of the `offset` field, hence `data` field
        let offsets_len = num_of_entry * size_of::<u16>();
        let data_len = len - offsets_len - size_of::<u16>();

        // Read data
        let b_data = data[0..data_len].to_vec();

        // Read offsets
        let mut offsets: Vec<u16> = Vec::with_capacity(num_of_entry);
        let mut cursor = &data[data_len..data_len + offsets_len];
        for _ in 0..num_of_entry {
            offsets.push(cursor.get_u16());
        }

        Self {
            data: b_data,
            offsets,
        }
    }
}
