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
use std::cmp::{max, min};

use crate::lsm_storage::LsmStorageState;

#[derive(Debug, Serialize, Deserialize)]
pub struct TieredCompactionTask {
    pub tiers: Vec<(usize, Vec<usize>)>,
    pub bottom_tier_included: bool,
}

#[derive(Debug, Clone)]
pub struct TieredCompactionOptions {
    pub num_tiers: usize,
    pub max_size_amplification_percent: usize,
    pub size_ratio: usize,
    pub min_merge_width: usize,
    pub max_merge_width: Option<usize>,
}

pub struct TieredCompactionController {
    options: TieredCompactionOptions,
}

impl TieredCompactionController {
    pub fn new(options: TieredCompactionOptions) -> Self {
        Self { options }
    }

    pub fn generate_compaction_task(
        &self,
        _snapshot: &LsmStorageState,
    ) -> Option<TieredCompactionTask> {
        if _snapshot.levels.len() < self.options.num_tiers {
            return None;
        }

        // Trigger 1: space amplification ratio
        // if all levels except last level size / last level size >= thresh
        // do a full compaction
        let level_cnt = _snapshot.levels.len();
        let all_levels_excxept_last_size: usize = (_snapshot.levels[..level_cnt - 1])
            .iter()
            .map(|(_, ids)| ids.len())
            .sum();
        let last_level_size = _snapshot.levels.last().unwrap().1.len();
        if last_level_size > 0
            && all_levels_excxept_last_size * 100 / last_level_size
                >= self.options.max_size_amplification_percent
        {
            return Some(TieredCompactionTask {
                tiers: _snapshot.levels.clone(),
                bottom_tier_included: true,
            });
        }

        // Trigger 2: size ratio
        // Scan tiers from newer->older, For first iter i where:
        // size(i) / sum(size(0..i-1)) > (100 + size_ratio)%
        // compact all previous tiers [0..i), excluding current tier i.
        let ratio_limit = 100 + self.options.size_ratio;
        let mut prev_sum = _snapshot.levels[0].1.len();

        for i in 1..level_cnt {
            let cur_size = _snapshot.levels[i].1.len();

            if prev_sum > 0
                && cur_size * 100 > prev_sum * ratio_limit
                && i >= self.options.min_merge_width
            {
                return Some(TieredCompactionTask {
                    tiers: _snapshot.levels[..i].to_vec(),
                    bottom_tier_included: false,
                });
            }

            prev_sum += cur_size;
        }

        // Trigger 3: number of tier
        // merge SST files from the first up to `max_merge_tiers` into one tier
        // this would help reduce total number of tiers
        let merge_width = self
            .options
            .max_merge_width
            .unwrap_or(_snapshot.levels.len())
            .min(_snapshot.levels.len());

        Some(TieredCompactionTask {
            tiers: _snapshot.levels[..merge_width].to_vec(),
            bottom_tier_included: merge_width == _snapshot.levels.len(),
        })
    }

    pub fn apply_compaction_result(
        &self,
        _snapshot: &LsmStorageState,
        _task: &TieredCompactionTask,
        _output: &[usize],
    ) -> (LsmStorageState, Vec<usize>) {
        let mut new_state = _snapshot.clone();

        // Inputs to delete from disk
        let files_to_remove: Vec<usize> = _task
            .tiers
            .iter()
            .flat_map(|(_, files)| files.iter().copied())
            .collect();

        // remove compacted files by tier id (not by index), so concurrent newly-flushed
        // tiers at the front are preserved
        let compacted_tier_ids: std::collections::HashSet<usize> =
            _task.tiers.iter().map(|(tier_id, _)| *tier_id).collect();

        new_state
            .levels
            .retain(|(tier_id, _)| !compacted_tier_ids.contains(tier_id));

        // Full compaction output becomes the new bottom tier
        // Tier id = first output sst id
        if let Some(&new_tier_id) = _output.first() {
            if _task.bottom_tier_included {
                new_state.levels.push((new_tier_id, _output.to_vec()));
            } else {
                // keep concurrent new tiers about the output
                let max_input_tier_id = _task.tiers.iter().map(|(id, _)| *id).max().unwrap_or(0);
                let insert_pos = new_state
                    .levels
                    .iter()
                    .take_while(|(tier_id, _)| *tier_id > max_input_tier_id)
                    .count();

                new_state
                    .levels
                    .insert(insert_pos, (new_tier_id, _output.to_vec()));
            }
        }

        (new_state, files_to_remove)
    }
}
