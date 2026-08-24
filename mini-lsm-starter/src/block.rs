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
        let mut buf = Vec::with_capacity(self.data.len() + self.offsets.len() * 2 + 2);
        buf.put_slice(&self.data);
        for &offset in &self.offsets {
            buf.put_u16_le(offset);
        }
        buf.put_u16_le(self.offsets.len() as u16);
        buf.into()
    }

    /// Decode from the data layout, transform the input `data` to a single `Block`
    ///
    /// Note: the decoder trusts its input; a production decoder would validate
    /// before indexing.
    pub fn decode(data: &[u8]) -> Self {
        // num_of_elements occupies the last 2 bytes.
        let mut num_bytes: &[u8] = &data[data.len() - 2..];
        let num_of_elements = num_bytes.get_u16_le() as usize;

        // The offset section (2 bytes per entry) sits just before num_of_elements.
        let offsets_end = data.len() - 2;
        let offsets_start = offsets_end - num_of_elements * 2;
        let offsets_raw = &data[offsets_start..offsets_end];
        let offsets: Vec<u16> = offsets_raw
            .chunks(2)
            .map(|mut chunk| chunk.get_u16_le())
            .collect();

        // Everything before the offset section is the data section.
        let data_section = &data[..offsets_start];
        Self {
            data: data_section.to_vec(),
            offsets,
        }
    }
}
