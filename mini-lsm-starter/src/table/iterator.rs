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

use std::mem;
use std::sync::Arc;

use anyhow::Result;

use super::SsTable;
use crate::block::Block;
use crate::{block::BlockIterator, iterators::StorageIterator, key::KeySlice};

/// An iterator over the contents of an SSTable.
pub struct SsTableIterator {
    table: Arc<SsTable>,
    blk_iter: BlockIterator,
    blk_idx: usize,
}

impl SsTableIterator {
    fn seek_to_key_inner(table: Arc<SsTable>, key: KeySlice) -> Result<Self> {
        let blk_idx = table.find_block_idx(key);
        if blk_idx >= table.num_of_blocks() {
            return Ok(Self {
                table,
                blk_idx,
                blk_iter: BlockIterator::create_and_seek_to_first(Arc::new(Block {
                    data: Vec::new(),
                    offsets: Vec::new(),
                })),
            });
        }

        let mut iter = Self {
            table: table.clone(),
            blk_idx,
            blk_iter: BlockIterator::create_and_seek_to_key(table.read_block(blk_idx)?, key),
        };
        if !iter.blk_iter.is_valid() { // not found by first key, so our iterator start at next block
            iter.blk_idx += 1;
            if iter.blk_idx < iter.table.num_of_blocks() {
                iter.blk_iter =
                    BlockIterator::create_and_seek_to_first(iter.table.read_block(iter.blk_idx)?);
            }
        }
        Ok(iter)
    }

    /// Create a new iterator and seek to the first key-value pair in the first data block.
    pub fn create_and_seek_to_first(table: Arc<SsTable>) -> Result<Self> {
        Ok(Self {
            table: table.clone(),
            blk_idx: 0,
            blk_iter: BlockIterator::create_and_seek_to_first(table.read_block(0)?),
        })
    }

    /// Seek to the first key-value pair in the first data block.
    pub fn seek_to_first(&mut self) -> Result<()> {
        self.blk_idx = 0;
        let _ = mem::replace(
            &mut self.blk_iter,
            BlockIterator::create_and_seek_to_first(self.table.read_block(0)?),
        );

        Ok(())
    }

    /// Create a new iterator and seek to the first key-value pair which >= `key`.
    pub fn create_and_seek_to_key(table: Arc<SsTable>, key: KeySlice) -> Result<Self> {
        Self::seek_to_key_inner(table, key)
    }

    /// Seek to the first key-value pair which >= `key`.
    /// Note: You probably want to review the handout for detailed explanation when implementing
    /// this function.
    pub fn seek_to_key(&mut self, key: KeySlice) -> Result<()> {
        let iter = Self::seek_to_key_inner(self.table.clone(), key)?;
        self.blk_idx = iter.blk_idx;
        self.blk_iter = iter.blk_iter;
        Ok(())
    }
}

impl StorageIterator for SsTableIterator {
    type KeyType<'a> = KeySlice<'a>;

    /// Return the `key` that's held by the underlying block iterator.
    fn key(&self) -> KeySlice<'_> {
        self.blk_iter.key()
    }

    /// Return the `value` that's held by the underlying block iterator.
    fn value(&self) -> &[u8] {
        self.blk_iter.value()
    }

    /// Return whether the current block iterator is valid or not.
    fn is_valid(&self) -> bool {
        self.blk_idx < self.table.num_of_blocks() && self.blk_iter.is_valid()
    }

    /// Move to the next `key` in the block.
    /// Note: You may want to check if the current block iterator is valid after the move.
    fn next(&mut self) -> Result<()> {
        self.blk_iter.next();

        // if this block is invalid, try to load to next block
        if !self.blk_iter.is_valid() {
            let next_blk_idx = self.blk_idx + 1;
            // no block left, do nothing then
            if next_blk_idx >= self.table.num_of_blocks() {
                return Ok(());
            }

            // have some block, update the block iterator then
            self.blk_idx = next_blk_idx;
            let new_block: Arc<Block> = self.table.read_block(next_blk_idx)?;
            let _ = mem::replace(
                &mut self.blk_iter,
                BlockIterator::create_and_seek_to_first(new_block),
            );

            return Ok(());
        }

        Ok(())
    }
}
