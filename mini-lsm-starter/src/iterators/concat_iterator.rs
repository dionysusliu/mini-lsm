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

use std::sync::Arc;

use anyhow::Result;

use super::StorageIterator;
use crate::{
    key::KeySlice,
    table::{SsTable, SsTableIterator},
};

/// Concat multiple iterators ordered in key order and their key ranges do not overlap. We do not want to create the
/// iterators when initializing this iterator to reduce the overhead of seeking.
pub struct SstConcatIterator {
    current: Option<SsTableIterator>,
    next_sst_idx: usize,
    sstables: Vec<Arc<SsTable>>,
}

impl SstConcatIterator {
    pub fn seek_to_next_sst(&mut self) -> Result<()> {
        while self.next_sst_idx < self.sstables.len() {
            let idx = self.next_sst_idx;
            self.next_sst_idx += 1;

            let iter = SsTableIterator::create_and_seek_to_first(Arc::clone(&self.sstables[idx]))?;
            if iter.is_valid() {
                self.current = Some(iter);
                return Ok(());
            }
        }

        self.current = None;
        Ok(())
    }

    pub fn create_and_seek_to_first(sstables: Vec<Arc<SsTable>>) -> Result<Self> {
        let mut iter = Self {
            current: None,
            next_sst_idx: 0,
            sstables,
        };

        iter.seek_to_next_sst()?;
        Ok(iter)
    }

    pub fn create_and_seek_to_key(sstables: Vec<Arc<SsTable>>, key: KeySlice) -> Result<Self> {
        if sstables.is_empty() {
            return Ok(Self {
                current: None,
                next_sst_idx: 0,
                sstables,
            });
        }

        // Find first SST whose last_key >= key
        // left is our target
        let (mut left, mut right) = (0_usize, sstables.len());
        while left < right {
            let mid = left + (right - left) / 2;
            if sstables[mid].last_key().as_key_slice() < key {
                left = mid + 1;
            } else {
                right = mid;
            }
        }

        let mut iter = Self {
            current: None,
            next_sst_idx: left.saturating_add(1),
            sstables,
        };
        if left >= iter.sstables.len() {
            return Ok(iter);
        }

        let sst_iter =
            SsTableIterator::create_and_seek_to_key(Arc::clone(&iter.sstables[left]), key)?;
        if sst_iter.is_valid() {
            // have some keys >= key
            iter.current = Some(sst_iter);
            return Ok(iter);
        }

        // all keys < key, so start from next sst
        iter.seek_to_next_sst()?;
        Ok(iter)
    }
}

impl StorageIterator for SstConcatIterator {
    type KeyType<'a> = KeySlice<'a>;

    fn key(&self) -> KeySlice {
        self.current.as_ref().unwrap().key()
    }

    fn value(&self) -> &[u8] {
        self.current.as_ref().unwrap().value()
    }

    fn is_valid(&self) -> bool {
        self.current.is_some()
    }

    fn next(&mut self) -> Result<()> {
        if let Some(iter) = self.current.as_mut() {
            iter.next()?;
            if iter.is_valid() {
                return Ok(());
            }
        }

        self.seek_to_next_sst()
    }

    fn num_active_iterators(&self) -> usize {
        1
    }
}
