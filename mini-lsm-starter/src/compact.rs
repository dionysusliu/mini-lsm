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
use crate::iterators::merge_iterator::MergeIterator;
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
    fn compact(&self, _task: &CompactionTask) -> Result<Vec<Arc<SsTable>>> {
        let (l0_sstables, l1_sstables) = match _task {
            CompactionTask::ForceFullCompaction {
                l0_sstables,
                l1_sstables,
            } => (l0_sstables, l1_sstables),
            _ => unreachable!(),
        };

        let snapshot = Arc::clone(&self.state.read());

        let mut iters: Vec<Box<SsTableIterator>> = Vec::new();
        for sst_id in l0_sstables.iter().chain(l1_sstables.iter()) {
            let table = Arc::clone(snapshot.sstables.get(sst_id).unwrap());
            let iter = SsTableIterator::create_and_seek_to_first(table)?;
            if iter.is_valid() {
                iters.push(Box::new(iter));
            }
        }

        let mut merge_iter = MergeIterator::create(iters);
        if !merge_iter.is_valid() {
            return Ok(Vec::new());
        }

        let mut outputs = Vec::new();
        let mut builder = SsTableBuilder::new(self.options.block_size);

        while merge_iter.is_valid() {
            let key = merge_iter.key();
            let value = merge_iter.value();

            if !value.is_empty() {
                // remove tombstones
                builder.add(key, value);
                if builder.estimated_size() >= self.options.target_sst_size {
                    // new sst table required
                    let sst_id = self.next_sst_id();
                    let old_builder =
                        mem::replace(&mut builder, SsTableBuilder::new(self.options.block_size));
                    // write new sst
                    let sst = old_builder.build(
                        sst_id,
                        Some(self.block_cache.clone()),
                        self.path_of_sst(sst_id),
                    )?;
                    outputs.push(Arc::new(sst));
                }
            }

            merge_iter.next()?;
        }

        // last sst to build
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

        // remove old files
        let (old_l0, old_l1) = match &task {
            CompactionTask::ForceFullCompaction {
                l0_sstables,
                l1_sstables,
            } => (l0_sstables.clone(), l1_sstables.clone()),
            _ => unreachable!(),
        };

        let old_l0_set: HashSet<usize> = old_l0.iter().copied().collect();

        // change pointers to l0 and l1 SSTs
        let files_to_remove: Vec<usize> = {
            // update state: no freeze, no flush
            let _state_lock = self.state_lock.lock();
            let mut snapshot = self.state.read().as_ref().clone();

            // keep newly flushed l0 during compactions
            snapshot.l0_sstables.retain(|id| !old_l0_set.contains(id));
            // replace l1 tables
            for sst in &new_ssts {
                snapshot.sstables.insert(sst.sst_id(), Arc::clone(sst));
            }
            snapshot.levels[0].1 = new_ids;

            // collect files to remove
            let mut removed = Vec::new();
            for id in old_l0.iter().chain(old_l1.iter()) {
                if snapshot.sstables.remove(id).is_some() {
                    removed.push(*id);
                }
            }

            // update states
            *self.state.write() = Arc::new(snapshot);

            removed
        };

        for id in files_to_remove {
            let _ = std::fs::remove_file(self.path_of_sst(id));
        }

        Ok(())
    }

    fn trigger_compaction(&self) -> Result<()> {
        self.force_full_compaction()
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
