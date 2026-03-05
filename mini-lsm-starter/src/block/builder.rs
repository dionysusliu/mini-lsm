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
        BlockBuilder {
            offsets: Vec::<u16>::new(),
            data: vec![],
            block_size,
            first_key: KeyVec::new(),
        }
    }

    /// Adds a key-value pair to the block. Returns false when the block is full.
    /// You may find the `bytes::BufMut` trait useful for manipulating binary data.
    #[must_use]
    pub fn add(&mut self, key: KeySlice, value: &[u8]) -> bool {
        let value_len: u16 = value.len() as u16;

        // key_overlap_len (u16) | rest_key_len (u16) | key (rest_key_len)
        let overlap_len = self
            .first_key
            .as_key_slice()
            .raw_ref()
            .iter()
            .zip(key.raw_ref().iter())
            .take_while(|(a, b)| a == b)
            .count();
        let rest_key_len = key.len() - overlap_len;

        let entry_size = 4 + rest_key_len + 2 + value.len();
        let footer_size = (self.offsets.len() + 1) * 2 + 2;

        // reject if full, unless this is the first entry
        if !self.is_empty() && self.data.len() + entry_size + footer_size > self.block_size {
            return false;
        }

        // record offset of this entry = current end and data section
        self.offsets.push(self.data.len() as u16);

        // record first key, is empty
        if self.first_key.is_empty() {
            self.first_key = key.to_key_vec();
        }

        // encode key, by comparing with the first key, and store as:
        self.data.put_u16(overlap_len as u16);
        self.data.put_u16(rest_key_len as u16);
        self.data.put_slice(&key.raw_ref()[overlap_len..]);

        // encode value
        self.data.put_u16(value_len);
        self.data.put_slice(value);

        true
    }

    /// Check if there is no key-value pair in the block.
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Finalize the block.
    pub fn build(self) -> Block {
        Block {
            data: self.data,
            offsets: self.offsets,
        }
    }
}
