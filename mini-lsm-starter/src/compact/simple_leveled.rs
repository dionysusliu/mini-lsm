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

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::lsm_storage::LsmStorageState;

#[derive(Debug, Clone)]
pub struct SimpleLeveledCompactionOptions {
    pub size_ratio_percent: usize,
    pub level0_file_num_compaction_trigger: usize,
    pub max_levels: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleLeveledCompactionTask {
    // if upper_level is `None`, then it is L0 compaction
    pub upper_level: Option<usize>,
    pub upper_level_sst_ids: Vec<usize>,
    pub lower_level: usize,
    pub lower_level_sst_ids: Vec<usize>,
    pub is_lower_level_bottom_level: bool,
}

pub struct SimpleLeveledCompactionController {
    options: SimpleLeveledCompactionOptions,
}

impl SimpleLeveledCompactionController {
    pub fn new(options: SimpleLeveledCompactionOptions) -> Self {
        Self { options }
    }

    /// Generates a compaction task.
    ///
    /// Returns `None` if no compaction needs to be scheduled. The order of SSTs in the compaction task id vector matters.
    pub fn generate_compaction_task(
        &self,
        _snapshot: &LsmStorageState,
    ) -> Option<SimpleLeveledCompactionTask> {
        // scan from L0 to Li-1
        // L0 applies table count trigger
        if _snapshot.l0_sstables.len() >= self.options.level0_file_num_compaction_trigger {
            return Some(SimpleLeveledCompactionTask {
                upper_level: None,
                upper_level_sst_ids: _snapshot.l0_sstables.clone(),
                lower_level: 1,
                lower_level_sst_ids: _snapshot.levels[0].1.clone(),
                is_lower_level_bottom_level: self.options.max_levels == 1,
            });
        }
        // Li applies ratio trigger.
        for upper_idx in 0..(self.options.max_levels.saturating_sub(1)) {
            let upper = &_snapshot.levels[upper_idx].1;
            if upper.is_empty() {
                continue;
            }
            let lower = &_snapshot.levels[upper_idx + 1].1;

            if lower.len() * 100 < upper.len() * self.options.size_ratio_percent {
                return Some(SimpleLeveledCompactionTask {
                    upper_level: Some(upper_idx + 1),
                    upper_level_sst_ids: _snapshot.levels[upper_idx].1.clone(),
                    lower_level: upper_idx + 2,
                    lower_level_sst_ids: _snapshot.levels[upper_idx + 1].1.clone(),
                    is_lower_level_bottom_level: (upper_idx + 2) == self.options.max_levels,
                });
            }
        }

        None
    }

    /// Apply the compaction result.
    ///
    /// The compactor will call this function with the compaction task and the list of SST ids generated. This function applies the
    /// result and generates a new LSM state. The functions should only change `l0_sstables` and `levels` without changing memtables
    /// and `sstables` hash map. Though there should only be one thread running compaction jobs, you should think about the case
    /// where an L0 SST gets flushed while the compactor generates new SSTs, and with that in mind, you should do some sanity checks
    /// in your implementation.
    pub fn apply_compaction_result(
        &self,
        _snapshot: &LsmStorageState,
        _task: &SimpleLeveledCompactionTask,
        _output: &[usize],
    ) -> (LsmStorageState, Vec<usize>) {
        let mut new_state = _snapshot.clone();

        let upper_set: HashSet<usize> = _task.upper_level_sst_ids.iter().copied().collect();
        let lower_set: HashSet<usize> = _task.lower_level_sst_ids.iter().copied().collect();

        let mut files_to_remove = Vec::new();
        files_to_remove.extend(_task.upper_level_sst_ids.iter().copied());
        files_to_remove.extend(_task.lower_level_sst_ids.iter().copied());

        // remove compacted upper inputs, keep concurrent new L0 flushed
        if let Some(upper_level) = _task.upper_level {
            let upper_idx = upper_level - 1;
            new_state.levels[upper_idx]
                .1
                .retain(|id| !upper_set.contains(id));
        } else {
            new_state.l0_sstables.retain(|id| !upper_set.contains(id));
        }

        // replace compacted lower layer with outputs (defensive retain+extend)
        let lower_idx = _task.lower_level - 1;
        new_state.levels[lower_idx]
            .1
            .retain(|id| !lower_set.contains(id));
        new_state.levels[lower_idx].1.extend_from_slice(_output);

        (new_state, files_to_remove)
    }
}
