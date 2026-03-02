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
//
//
// goals
// 1. add

#![allow(unused_variables)] // TODO(you): remove this lint after implementing this mod
#![allow(dead_code)] // TODO(you): remove this lint after implementing this mod

use bytes::BufMut;
use std::sync::Arc;
use std::{mem, path::Path};

use anyhow::Result;

use super::{BlockMeta, SsTable};
use crate::block;
use crate::key::KeyBytes;
use crate::table::FileObject;
use crate::{block::BlockBuilder, key::KeySlice, lsm_storage::BlockCache};

/// Builds an SSTable from key-value pairs.
pub struct SsTableBuilder {
    builder: BlockBuilder,
    first_key: Vec<u8>,
    last_key: Vec<u8>,
    data: Vec<u8>,
    pub(crate) meta: Vec<BlockMeta>,
    block_size: usize,
}

impl SsTableBuilder {
    /// Create a builder based on target block size.
    pub fn new(block_size: usize) -> Self {
        SsTableBuilder {
            builder: BlockBuilder::new(block_size),
            first_key: vec![],
            last_key: vec![],
            data: vec![],
            meta: vec![],
            block_size,
        }
    }

    /// Adds a key-value pair to SSTable.
    ///
    /// Note: You should split a new block when the current block is full.(`std::mem::replace` may
    /// be helpful here)
    pub fn add(&mut self, key: KeySlice, value: &[u8]) {
        // add to builder
        // determine whether use new block
        if !self.builder.add(key, value) {
            // replace builder with new one
            // move data ownership out
            let this_builder = mem::replace(&mut self.builder, BlockBuilder::new(self.block_size));
            let mut block_data: Vec<u8> = this_builder
                .build()
                .encode()
                .try_into_mut()
                .expect("freshly created, should be sole owner")
                .into();

            // dump block meta to meta
            let this_first_key = mem::take(&mut self.first_key); // move data ownership out
            let this_last_key = mem::take(&mut self.last_key);
            let this_meta = BlockMeta {
                offset: self.data.len(),
                first_key: KeyBytes::from_bytes(this_first_key.into()),
                last_key: KeyBytes::from_bytes(this_last_key.into()),
            };
            self.meta.push(this_meta);

            // dump data
            self.data.append(&mut block_data);

            // add new key to block builder
            let _ = self.builder.add(key, value);
        }

        // update first key, if empty
        if self.first_key.is_empty() {
            self.first_key = key.raw_ref().to_vec();
        }
        // update last key
        self.last_key.clear();
        self.last_key.extend_from_slice(key.raw_ref());
    }

    /// Get the estimated size of the SSTable.
    ///
    /// Since the data blocks contain much more data than meta blocks, just return the size of data
    /// blocks here.
    pub fn estimated_size(&self) -> usize {
        self.data.len() + self.block_size
    }

    /// Builds the SSTable and writes it to the given path. Use the `FileObject` structure to manipulate the disk objects.
    pub fn build(
        #[allow(unused_mut)] mut self,
        id: usize,
        block_cache: Option<Arc<BlockCache>>,
        path: impl AsRef<Path>,
    ) -> Result<SsTable> {
        // flush the last inprogress block
        let last_builder = mem::replace(&mut self.builder, BlockBuilder::new(self.block_size));
        let mut last_block = last_builder.build();
        let last_meta = BlockMeta {
            offset: self.data.len(),
            first_key: KeyBytes::from_bytes(mem::take(&mut self.first_key).into()),
            last_key: KeyBytes::from_bytes(mem::take(&mut self.last_key).into()),
        };
        self.meta.push(last_meta);
        self.data.append(&mut last_block.data);

        // record where meta section starts
        let block_meta_offset = self.data.len();

        // encode block metas
        BlockMeta::encode_block_meta(&self.meta, &mut self.data);

        // encode meta block offset
        self.data.put_u32(block_meta_offset as u32);

        // write to disk
        let file = FileObject::create(path.as_ref(), self.data)?;

        // extract SSTable-level first/last keys from meta
        let first_key = self.meta.first().unwrap().first_key.clone();
        let last_key = self.meta.last().unwrap().last_key.clone();

        Ok(SsTable {
            file,
            block_meta: self.meta,
            block_meta_offset,
            id,
            block_cache,
            first_key,
            last_key,
            bloom: None,
            max_ts: 0,
        })
    }

    #[cfg(test)]
    pub(crate) fn build_for_test(self, path: impl AsRef<Path>) -> Result<SsTable> {
        self.build(0, None, path)
    }
}
