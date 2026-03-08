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
use std::cmp::min;
use std::collections::HashSet;

use crate::lsm_storage::LsmStorageState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeveledCompactionTask {
    // if upper_level is `None`, then it is L0 compaction
    pub upper_level: Option<usize>,
    pub upper_level_sst_ids: Vec<usize>,
    pub lower_level: usize,
    pub lower_level_sst_ids: Vec<usize>,
    pub is_lower_level_bottom_level: bool,
}

#[derive(Debug, Clone)]
pub struct LeveledCompactionOptions {
    pub level_size_multiplier: usize,
    pub level0_file_num_compaction_trigger: usize,
    pub max_levels: usize,
    pub base_level_size_mb: usize,
}

pub struct LeveledCompactionController {
    options: LeveledCompactionOptions,
}

impl LeveledCompactionController {
    pub fn new(options: LeveledCompactionOptions) -> Self {
        Self { options }
    }

    /// Compute actual level sizes
    fn level_real_sizes(&self, snapshot: &LsmStorageState) -> Vec<u64> {
        snapshot
            .levels
            .iter()
            .map(|(_, sst_ids)| {
                sst_ids
                    .iter()
                    .map(|id| snapshot.sstables.get(id).unwrap().table_size())
                    .sum::<u64>()
            })
            .collect()
    }

    /// Target size for each level in snapshot.levels (L1...Lmax), in bytes
    fn compute_target_size(&self, snapshot: &LsmStorageState) -> Vec<u64> {
        let n = snapshot.levels.len();
        if n == 0 {
            return Vec::new();
        }

        let real_sizes = self.level_real_sizes(snapshot);
        let base = self.options.base_level_size_mb as u64 * 1024 * 1024;
        let mul = self.options.level_size_multiplier.max(1) as u64;

        let mut target = vec![0u64; n];
        let last = n - 1;
        let bottom_real = real_sizes[last];

        if bottom_real < base {
            target[last] = base;
            return target;
        }

        // Assign new target sizes
        // At most one level may have 0 < target < base
        target[last] = bottom_real;
        for i in (0..last).rev() {
            target[i] = target[i + 1] / mul;
            if target[i] < base {
                break;
            }
        }

        target
    }

    fn find_overlapping_ssts(
        &self,
        _snapshot: &LsmStorageState,
        _sst_ids: &[usize],
        _in_level: usize,
    ) -> Vec<usize> {
        if _sst_ids.is_empty() {
            return Vec::new();
        }

        // Key span of compaction input
        let (mut min_key, mut max_key) = (None, None);
        for sst_id in _sst_ids {
            let table = _snapshot.sstables.get(sst_id).unwrap();
            let first = table.first_key();
            let last = table.last_key();

            min_key = Some(match min_key {
                Some(cur) => std::cmp::min(cur, first),
                None => first,
            });
            max_key = Some(match max_key {
                Some(cur) => std::cmp::max(cur, last),
                None => last,
            });
        }

        let min_key = min_key.unwrap();
        let max_key = max_key.unwrap();

        _snapshot.levels[_in_level - 1]
            .1
            .iter()
            .copied()
            .filter(|id| {
                let table = _snapshot.sstables.get(id).unwrap();
                !(table.last_key() < min_key || table.first_key() > max_key)
            })
            .collect()
    }

    fn decide_base_level(&self, snapshot: &LsmStorageState) -> usize {
        let targets = self.compute_target_size(snapshot);
        targets
            .iter()
            .position(|&x| x > 0)
            .map(|idx| idx + 1)
            .unwrap_or(self.options.max_levels)
    }

    pub fn generate_compaction_task(
        &self,
        _snapshot: &LsmStorageState,
    ) -> Option<LeveledCompactionTask> {
        // L0 branch
        if _snapshot.l0_sstables.len() >= self.options.level0_file_num_compaction_trigger {
            let lower_level = self.decide_base_level(_snapshot);
            let lower_level_sst_ids =
                self.find_overlapping_ssts(_snapshot, &_snapshot.l0_sstables, lower_level);

            return Some(LeveledCompactionTask {
                upper_level: None,
                upper_level_sst_ids: _snapshot.l0_sstables.clone(),
                lower_level,
                lower_level_sst_ids,
                is_lower_level_bottom_level: lower_level == self.options.max_levels,
            });
        }

        // L1 .. Lmax
        let real_sizes = self.level_real_sizes(_snapshot);
        let targets = self.compute_target_size(_snapshot);

        let mut best_level_idx = None;
        let mut best_score = 1.0f64;
        for i in 0..(self.options.max_levels - 1) {
            let target = targets[i];
            if target == 0 {
                continue;
            }
            let score = real_sizes[i] as f64 / target as f64;
            if score > best_score {
                best_score = score;
                best_level_idx = Some(i);
            }
        }

        let upper_idx = best_level_idx?;
        let upper_sst_id = match _snapshot.levels[upper_idx].1.first() {
            Some(id) => *id,
            None => return None,
        };

        let upper_level_sst_ids = vec![upper_sst_id];
        let lower_level = upper_idx + 2;
        let lower_level_sst_ids =
            self.find_overlapping_ssts(_snapshot, &upper_level_sst_ids, lower_level);

        Some(LeveledCompactionTask {
            upper_level: Some(upper_idx + 1),
            upper_level_sst_ids,
            lower_level,
            lower_level_sst_ids,
            is_lower_level_bottom_level: lower_level == self.options.max_levels,
        })
    }

    pub fn apply_compaction_result(
        &self,
        _snapshot: &LsmStorageState,
        _task: &LeveledCompactionTask,
        _output: &[usize],
        _in_recovery: bool,
    ) -> (LsmStorageState, Vec<usize>) {
        let mut new_state = _snapshot.clone();

        let upper_set: HashSet<usize> = _task.upper_level_sst_ids.iter().copied().collect();
        let lower_set: HashSet<usize> = _task.lower_level_sst_ids.iter().copied().collect();

        let mut files_to_remove = Vec::new();
        files_to_remove.extend(_task.upper_level_sst_ids.iter().copied());
        files_to_remove.extend(_task.lower_level_sst_ids.iter().copied());

        // Remove upper inputs
        if let Some(upper_level) = _task.upper_level {
            let upper_idx = upper_level - 1;
            new_state.levels[upper_idx]
                .1
                .retain(|id| !upper_set.contains(id));
        } else {
            // L0 compaction
            new_state.l0_sstables.retain(|id| !upper_set.contains(id));
        }

        // Replace lower overlaps with output
        let lower_idx = _task.lower_level - 1;
        new_state.levels[lower_idx]
            .1
            .retain(|id| !lower_set.contains(id));
        if !_output.is_empty() {
            if _in_recovery {
                // no output SST metadata yet during replay 
                new_state.levels[lower_idx].1.extend_from_slice(_output);
            } else {
                // Derive insertion anchor from upper input key range (always exists in snapshot).
                let upper_min = _task
                    .upper_level_sst_ids
                    .iter()
                    .map(|id| _snapshot.sstables.get(id).unwrap().first_key())
                    .min()
                    .unwrap();
                let insert_pos = new_state.levels[lower_idx]
                    .1
                    .iter()
                    .position(|id| _snapshot.sstables.get(id).unwrap().last_key() >= upper_min)
                    .unwrap_or(new_state.levels[lower_idx].1.len());

                new_state.levels[lower_idx]
                    .1
                    .splice(insert_pos..insert_pos, _output.iter().copied());
            }
            
        }

        (new_state, files_to_remove)
    }
}
