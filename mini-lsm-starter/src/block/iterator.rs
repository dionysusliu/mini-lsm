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

// #![allow(unused_variables)] // TODO(you): remove this lint after implementing this mod
// #![allow(dead_code)] // TODO(you): remove this lint after implementing this mod

use std::sync::Arc;

use crate::key::{KeySlice, KeyVec};
use bytes::Buf;

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
        if block.offsets.is_empty() {
            return Self::new(block);
        }

        let mut iter = Self::new(block);
        //  first key
        iter.set_first_key();
        //  states
        iter.seek_to_first();

        iter
    }

    /// Creates a block iterator and seek to the first key that >= `key`.
    pub fn create_and_seek_to_key(block: Arc<Block>, key: KeySlice) -> Self {
        if block.offsets.is_empty() {
            return Self::new(block);
        }

        let mut iter = Self::new(block);
        //  first key
        iter.set_first_key();
        //  states
        iter.seek_to_key(key);

        iter
    }

    /// Returns the key of the current entry.
    pub fn key(&self) -> KeySlice<'_> {
        self.key.as_key_slice()
    }

    /// Returns the value of the current entry.
    pub fn value(&self) -> &[u8] {
        let (val_start, val_end) = self.value_range;
        &self.block.data[val_start..val_end]
    }

    /// Returns true if the iterator is valid.
    /// Note: You may want to make use of `key`
    pub fn is_valid(&self) -> bool {
        !self.key().is_empty()
    }

    /// Seeks to the first key in the block.
    pub fn seek_to_first(&mut self) {
        self.set_to_ith_key(0);
    }

    /// Move to the next key in the block.
    pub fn next(&mut self) {
        if self.is_end_of_block() {
            self.key = KeyVec::new();
            return;
        }

        // idx
        self.idx += 1;
        self.set_to_ith_key(self.idx);
    }

    /// Seek to the first key that >= `key`.
    /// Note: You should assume the key-value pairs in the block are sorted when being added by
    /// callers.
    pub fn seek_to_key(&mut self, key: KeySlice) {
        // binary search on offset space
        let (mut left, mut right) = (0_usize, self.block.offsets.len());
        while left < right {
            let mid = left + (right - left) / 2;
            if self.check_ge_at(mid, key) {
                right = mid;
            } else {
                left = mid + 1;
            }
        }

        // left is the target idx, check if valid
        if left >= self.block.offsets.len() {
            self.key = KeyVec::new();
            return;
        }

        self.set_to_ith_key(left);
    }

    /// Current iterator (cursor) is at the end of block (last key)
    fn is_end_of_block(&self) -> bool {
        self.idx + 1 >= self.block.offsets.len()
    }

    /// return true if the idx-th key >= input key
    fn check_ge_at(&self, idx: usize, key: KeySlice) -> bool {
        let kv_offset = self.block.offsets[idx] as usize;
        let mut buf = &self.block.data[kv_offset..];
        // key
        let key_len = buf.get_u16() as usize;
        let key_slice = KeySlice::from_slice(&buf[..key_len]);

        key_slice >= key
    }

    /// set iterator at i-th KV pair, where i in [0, number_of_elements)
    fn set_to_ith_key(&mut self, i: usize) {
        // set iterator state
        let kv_offset = self.block.offsets[i] as usize;
        // key
        let mut buf = &self.block.data[kv_offset..];
        let key_len = buf.get_u16() as usize;
        self.key
            .set_from_slice(KeySlice::from_slice(&buf[..key_len]));
        buf.advance(key_len);
        // value range
        let val_len = buf.get_u16() as usize;
        let val_start = kv_offset + 4 + key_len;
        self.value_range = (val_start, val_start + val_len);
        // idx
        self.idx = i;
    }

    /// called once on creation, set the first key field
    fn set_first_key(&mut self) {
        let mut buf = &self.block.data[..];
        let key_len = buf.get_u16() as usize;
        self.first_key
            .set_from_slice(KeySlice::from_slice(&buf[..key_len]));
    }
}
