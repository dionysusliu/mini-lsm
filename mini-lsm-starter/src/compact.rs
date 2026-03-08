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

mod leveled;
mod simple_leveled;
mod tiered;

use std::collections::HashSet;
use std::mem;
use std::sync::Arc;
use std::time::Duration;

use crate::iterators::StorageIterator;
use crate::iterators::concat_iterator::SstConcatIterator;
use crate::iterators::merge_iterator::MergeIterator;
use crate::iterators::two_merge_iterator::TwoMergeIterator;
use crate::key::KeySlice;
use crate::lsm_storage::{LsmStorageInner, LsmStorageState};
use crate::table::{SsTable, SsTableBuilder, SsTableIterator};
use anyhow::Result;
pub use leveled::{LeveledCompactionController, LeveledCompactionOptions, LeveledCompactionTask};
use serde::{Deserialize, Serialize};
pub use simple_leveled::{
    SimpleLeveledCompactionController, SimpleLeveledCompactionOptions, SimpleLeveledCompactionTask,
};
pub use tiered::{TieredCompactionController, TieredCompactionOptions, TieredCompactionTask};

#[derive(Debug, Serialize, Deserialize)]
pub enum CompactionTask {
    Leveled(LeveledCompactionTask),
    Tiered(TieredCompactionTask),
    Simple(SimpleLeveledCompactionTask),
    ForceFullCompaction {
        l0_sstables: Vec<usize>,
        l1_sstables: Vec<usize>,
    },
}

impl CompactionTask {
    fn compact_to_bottom_level(&self) -> bool {
        match self {
            CompactionTask::ForceFullCompaction { .. } => true,
            CompactionTask::Leveled(task) => task.is_lower_level_bottom_level,
            CompactionTask::Simple(task) => task.is_lower_level_bottom_level,
            CompactionTask::Tiered(task) => task.bottom_tier_included,
        }
    }
}

pub(crate) enum CompactionController {
    Leveled(LeveledCompactionController),
    Tiered(TieredCompactionController),
    Simple(SimpleLeveledCompactionController),
    NoCompaction,
}

impl CompactionController {
    pub fn generate_compaction_task(&self, snapshot: &LsmStorageState) -> Option<CompactionTask> {
        match self {
            CompactionController::Leveled(ctrl) => ctrl
                .generate_compaction_task(snapshot)
                .map(CompactionTask::Leveled),
            CompactionController::Simple(ctrl) => ctrl
                .generate_compaction_task(snapshot)
                .map(CompactionTask::Simple),
            CompactionController::Tiered(ctrl) => ctrl
                .generate_compaction_task(snapshot)
                .map(CompactionTask::Tiered),
            CompactionController::NoCompaction => unreachable!(),
        }
    }

    pub fn apply_compaction_result(
        &self,
        snapshot: &LsmStorageState,
        task: &CompactionTask,
        output: &[usize],
        in_recovery: bool,
    ) -> (LsmStorageState, Vec<usize>) {
        match (self, task) {
            (CompactionController::Leveled(ctrl), CompactionTask::Leveled(task)) => {
                ctrl.apply_compaction_result(snapshot, task, output, in_recovery)
            }
            (CompactionController::Simple(ctrl), CompactionTask::Simple(task)) => {
                ctrl.apply_compaction_result(snapshot, task, output)
            }
            (CompactionController::Tiered(ctrl), CompactionTask::Tiered(task)) => {
                ctrl.apply_compaction_result(snapshot, task, output)
            }
            _ => unreachable!(),
        }
    }
}

impl CompactionController {
    pub fn flush_to_l0(&self) -> bool {
        matches!(
            self,
            Self::Leveled(_) | Self::Simple(_) | Self::NoCompaction
        )
    }
}

#[derive(Debug, Clone)]
pub enum CompactionOptions {
    /// Leveled compaction with partial compaction + dynamic level support (= RocksDB's Leveled
    /// Compaction)
    Leveled(LeveledCompactionOptions),
    /// Tiered compaction (= RocksDB's universal compaction)
    Tiered(TieredCompactionOptions),
    /// Simple leveled compaction
    Simple(SimpleLeveledCompactionOptions),
    /// In no compaction mode (week 1), always flush to L0
    NoCompaction,
}

impl LsmStorageInner {
    /// generate new sst files from the storage iterator
    fn build_ssts_from_iter<I>(
        &self,
        mut iter: I,
        compact_to_bottom: bool,
    ) -> Result<Vec<Arc<SsTable>>>
    where
        I: for<'a> StorageIterator<KeyType<'a> = KeySlice<'a>>,
    {
        let mut outputs = Vec::new();
        let mut builder = SsTableBuilder::new(self.options.block_size);

        while iter.is_valid() {
            let key = iter.key();
            let value = iter.value();

            // only drop tombstones when compacting to bottommost level
            if !(compact_to_bottom && value.is_empty()) {
                builder.add(key, value);
                if builder.estimated_size() >= self.options.target_sst_size {
                    let sst_id = self.next_sst_id();
                    let old_builder =
                        mem::replace(&mut builder, SsTableBuilder::new(self.options.block_size));
                    let sst = old_builder.build(
                        sst_id,
                        Some(self.block_cache.clone()),
                        self.path_of_sst(sst_id),
                    )?;
                    outputs.push(Arc::new(sst));
                }
            }

            iter.next()?;
        }

        if !builder.is_empty() {
            let sst_id = self.next_sst_id();
            let sst = builder.build(
                sst_id,
                Some(self.block_cache.clone()),
                self.path_of_sst(sst_id),
            )?;
            outputs.push(Arc::new(sst));
        }

        Ok(outputs)
    }

    /// compact L0 with L1
    fn compact_l0_to_l1(
        &self,
        snapshot: &Arc<LsmStorageState>,
        l0_sstables: &[usize],
        l1_sstables: &[usize],
        compact_to_bottom: bool,
    ) -> Result<Vec<Arc<SsTable>>> {
        let mut l0_iters = Vec::new();
        for sst_id in l0_sstables {
            let table = Arc::clone(snapshot.sstables.get(sst_id).unwrap());
            let iter = SsTableIterator::create_and_seek_to_first(table)?;
            if iter.is_valid() {
                l0_iters.push(Box::new(iter));
            }
        }
        let l0_merge = MergeIterator::create(l0_iters);

        let l1_tables: Vec<Arc<SsTable>> = l1_sstables
            .iter()
            .map(|sst_id| Arc::clone(snapshot.sstables.get(sst_id).unwrap()))
            .collect();
        let mut l1_iters = Vec::new();
        if !l1_tables.is_empty() {
            let iter = SstConcatIterator::create_and_seek_to_first(l1_tables)?;
            if iter.is_valid() {
                l1_iters.push(Box::new(iter));
            }
        }
        let l1_merge = MergeIterator::create(l1_iters);

        let iter = TwoMergeIterator::create(l0_merge, l1_merge)?;
        self.build_ssts_from_iter(iter, compact_to_bottom)
    }

    /// compact Li with Li+1, where i >= 1
    /// using concat iterator to avoid unnecessary block loads
    fn compact_level_to_level(
        &self,
        snapshot: &Arc<LsmStorageState>,
        upper_sstables: &[usize],
        lower_sstables: &[usize],
        compact_to_bottom: bool,
    ) -> Result<Vec<Arc<SsTable>>> {
        let upper_tables: Vec<Arc<SsTable>> = upper_sstables
            .iter()
            .map(|sst_id| Arc::clone(snapshot.sstables.get(sst_id).unwrap()))
            .collect();
        let mut upper_iters = Vec::new();
        if !upper_tables.is_empty() {
            let iter = SstConcatIterator::create_and_seek_to_first(upper_tables)?;
            if iter.is_valid() {
                upper_iters.push(Box::new(iter));
            }
        }
        let upper_merge = MergeIterator::create(upper_iters);

        let lower_tables: Vec<Arc<SsTable>> = lower_sstables
            .iter()
            .map(|sst_id| Arc::clone(snapshot.sstables.get(sst_id).unwrap()))
            .collect();
        let mut lower_iters = Vec::new();
        if !lower_tables.is_empty() {
            let iter = SstConcatIterator::create_and_seek_to_first(lower_tables)?;
            if iter.is_valid() {
                lower_iters.push(Box::new(iter));
            }
        }
        let lower_merge = MergeIterator::create(lower_iters);

        let iter = TwoMergeIterator::create(upper_merge, lower_merge)?;
        self.build_ssts_from_iter(iter, compact_to_bottom)
    }

    /// compact multiple levels (tiers/sorted runs)
    fn compact_tiered(
        &self,
        snapshot: &LsmStorageState,
        tiers: &[(usize, Vec<usize>)],
        compact_to_bottom: bool,
    ) -> Result<Vec<Arc<SsTable>>> {
        // construct iterators
        let mut tier_iters: Vec<Box<SstConcatIterator>> = Vec::new();
        for (_tier_id, sst_ids) in tiers {
            let tier_tables: Vec<Arc<SsTable>> = sst_ids
                .iter()
                .map(|sst_id| snapshot.sstables.get(sst_id).unwrap().clone())
                .collect();

            if tier_tables.is_empty() {
                continue;
            }

            let iter = SstConcatIterator::create_and_seek_to_first(tier_tables)?;
            if iter.is_valid() {
                tier_iters.push(Box::new(iter));
            }
        }

        let merged = MergeIterator::create(tier_iters);

        self.build_ssts_from_iter(merged, compact_to_bottom)
    }

    fn compact(&self, task: &CompactionTask) -> Result<Vec<Arc<SsTable>>> {
        let snapshot = Arc::clone(&self.state.read());
        let compact_to_bottom = task.compact_to_bottom_level();

        match task {
            CompactionTask::ForceFullCompaction {
                l0_sstables,
                l1_sstables,
            } => self.compact_l0_to_l1(&snapshot, l0_sstables, l1_sstables, compact_to_bottom),
            CompactionTask::Simple(task) if task.upper_level.is_none() => self.compact_l0_to_l1(
                &snapshot,
                &task.upper_level_sst_ids,
                &task.lower_level_sst_ids,
                compact_to_bottom,
            ),
            CompactionTask::Simple(task) => self.compact_level_to_level(
                &snapshot,
                &task.upper_level_sst_ids,
                &task.lower_level_sst_ids,
                compact_to_bottom,
            ),
            CompactionTask::Tiered(task) => {
                self.compact_tiered(&snapshot, &task.tiers, compact_to_bottom)
            }
            CompactionTask::Leveled(task) if task.upper_level.is_none() => self.compact_l0_to_l1(
                &snapshot,
                &task.upper_level_sst_ids,
                &task.lower_level_sst_ids,
                compact_to_bottom,
            ),
            CompactionTask::Leveled(task) => self.compact_level_to_level(
                &snapshot,
                &task.upper_level_sst_ids,
                &task.lower_level_sst_ids,
                compact_to_bottom,
            ),
            _ => unimplemented!(),
        }
    }

    pub fn force_full_compaction(&self) -> Result<()> {
        let snapshot = Arc::clone(&self.state.read());
        let task = CompactionTask::ForceFullCompaction {
            l0_sstables: snapshot.l0_sstables.clone(),
            l1_sstables: snapshot.levels[0].1.clone(),
        };

        if let CompactionTask::ForceFullCompaction {
            l0_sstables,
            l1_sstables,
        } = &task
        {
            if l0_sstables.is_empty() && l1_sstables.is_empty() {
                return Ok(());
            }
        }

        // do compaction work
        let new_ssts = self.compact(&task)?;
        let new_ids: Vec<usize> = new_ssts.iter().map(|t| t.sst_id()).collect();

        // merge compaction results into current state
        self.install_compaction_result(snapshot, &task, &new_ssts, &new_ids)?;

        Ok(())
    }

    fn trigger_compaction(&self) -> Result<()> {
        let snapshot = Arc::clone(&self.state.read());
        let Some(task) = self
            .compaction_controller
            .generate_compaction_task(snapshot.as_ref())
        else {
            return Ok(());
        };

        let new_ssts = self.compact(&task)?;
        let output_ids: Vec<usize> = new_ssts.iter().map(|t| t.sst_id()).collect();
        self.install_compaction_result(snapshot, &task, &new_ssts, &output_ids)?;

        Ok(())
    }

    /// update the in-memory and disk state with compaction results
    fn install_compaction_result(
        &self,
        base_snapshot: Arc<LsmStorageState>,
        task: &CompactionTask,
        new_ssts: &[Arc<SsTable>],
        output_ids: &[usize],
    ) -> Result<Vec<usize>> {
        let files_to_remove = {
            let _state_lock = self.state_lock.lock();
            let latest = Arc::clone(&self.state.read());

            // apply compaction results to a current state snapshot, get (new_state, files_to_remove)
            let (mut new_state, files_to_remove) = match task {
                CompactionTask::ForceFullCompaction {
                    l0_sstables,
                    l1_sstables,
                } => {
                    let old_l0_set: HashSet<usize> = l0_sstables.iter().copied().collect();

                    let mut s = latest.as_ref().clone();
                    // change sst_id vectors, and sst hashmap
                    s.l0_sstables.retain(|id| !old_l0_set.contains(id));
                    s.levels[0].1 = output_ids.to_vec();

                    let mut removed = Vec::new();
                    for id in l0_sstables.iter().chain(l1_sstables.iter()) {
                        if s.sstables.remove(id).is_some() {
                            removed.push(*id);
                        }
                    }

                    (s, removed)
                }

                _ => self.compaction_controller.apply_compaction_result(
                    latest.as_ref(),
                    task,
                    output_ids,
                    false,
                ),
            };

            // update
            for sst in new_ssts {
                new_state.sstables.insert(sst.sst_id(), Arc::clone(sst));
            }
            for id in &files_to_remove {
                new_state.sstables.remove(id);
            }

            *self.state.write() = Arc::new(new_state);
            files_to_remove
        };

        for id in &files_to_remove {
            let _ = std::fs::remove_file(self.path_of_sst(*id));
        }
        Ok(files_to_remove)
    }

    pub(crate) fn spawn_compaction_thread(
        self: &Arc<Self>,
        rx: crossbeam_channel::Receiver<()>,
    ) -> Result<Option<std::thread::JoinHandle<()>>> {
        if let CompactionOptions::Leveled(_)
        | CompactionOptions::Simple(_)
        | CompactionOptions::Tiered(_) = self.options.compaction_options
        {
            let this = self.clone();
            let handle = std::thread::spawn(move || {
                let ticker = crossbeam_channel::tick(Duration::from_millis(50));
                loop {
                    crossbeam_channel::select! {
                        recv(ticker) -> _ => if let Err(e) = this.trigger_compaction() {
                            eprintln!("compaction failed: {}", e);
                        },
                        recv(rx) -> _ => return
                    }
                }
            });
            return Ok(Some(handle));
        }
        Ok(None)
    }

    fn trigger_flush(&self) -> Result<()> {
        if self.state.read().imm_memtables.len() + 1 > self.options.num_memtable_limit {
            self.force_flush_next_imm_memtable()?;
        }

        Ok(())
    }

    pub(crate) fn spawn_flush_thread(
        self: &Arc<Self>,
        rx: crossbeam_channel::Receiver<()>,
    ) -> Result<Option<std::thread::JoinHandle<()>>> {
        let this = self.clone();
        let handle = std::thread::spawn(move || {
            let ticker = crossbeam_channel::tick(Duration::from_millis(50));
            loop {
                crossbeam_channel::select! {
                    recv(ticker) -> _ => if let Err(e) = this.trigger_flush() {
                        eprintln!("flush failed: {}", e);
                    },
                    recv(rx) -> _ => return
                }
            }
        });
        Ok(Some(handle))
    }
}
