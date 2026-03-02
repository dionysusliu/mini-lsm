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

mod builder;
mod iterator;

pub use builder::BlockBuilder;
use bytes::{BufMut, Bytes};
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

        // data
        buf.put_slice(self.data.as_ref());
        // offset
        for offset in &self.offsets {
            buf.put_u16(*offset);
        }
        // extra
        buf.put_u16(self.offsets.len() as u16);

        Bytes::from(buf)
    }

    /// Decode from the data layout, transform the input `data` to a single `Block`
    pub fn decode(data: &[u8]) -> Self {
        // read extra
        let num_of_elements =
            u16::from_be_bytes(data[data.len() - 2..].try_into().unwrap()) as usize;
        // offsets
        let offset_end = data.len() - 2;
        let offset_start = offset_end - num_of_elements * 2;
        let offsets: Vec<u16> = data[offset_start..offset_end]
            .chunks_exact(2)
            .map(|b| u16::from_be_bytes(b.try_into().unwrap()))
            .collect();

        // data
        let data = Bytes::copy_from_slice(&data[..offset_start]).to_vec();

        Block { data, offsets }
    }
}
