// Copyright (c) 2022-2026 Alex Chi Z
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

use crate::lsm_storage::LsmStorageState;

#[derive(Debug, Clone)]
pub struct SimpleLeveledCompactionOptions {
    pub size_ratio_percent: usize,
    pub level0_file_num_compaction_trigger: usize,
    pub max_levels: usize,
}

#[derive(Debug, Serialize, Deserialize)]
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
    ///
    /// Policy (one task per call; the caller re-invokes until None):
    /// 1. L0 has >= level0_file_num_compaction_trigger files -> L0->L1 (L0 is
    ///    always first priority: new data lands there, and a blocked L0 spikes
    ///    read amplification immediately).
    /// 2. Otherwise, top-down, the FIRST adjacent pair whose lower level is
    ///    "too thin": (len(lower) / len(upper)) * 100 < size_ratio_percent.
    ///    Note the operand direction — lower over upper. Reversed operands
    ///    would compact healthy levels (book ch. question).
    /// 3. Nothing violated -> None (the convergence exit).
    pub fn generate_compaction_task(
        &self,
        snapshot: &LsmStorageState,
    ) -> Option<SimpleLeveledCompactionTask> {
        if snapshot.l0_sstables.len() >= self.options.level0_file_num_compaction_trigger {
            return Some(SimpleLeveledCompactionTask {
                upper_level: None,
                upper_level_sst_ids: snapshot.l0_sstables.clone(),
                lower_level: 1,
                lower_level_sst_ids: snapshot.levels[0].1.clone(),
                is_lower_level_bottom_level: self.options.max_levels == 1,
            });
        }
        // levels is a 0-based Vec: levels[i] = Li+1. Scan upper from L1 down;
        // the bottom level has no lower partner and never triggers.
        for upper_idx in 0..snapshot.levels.len().saturating_sub(1) {
            let upper_len = snapshot.levels[upper_idx].1.len();
            let lower_len = snapshot.levels[upper_idx + 1].1.len();
            if lower_len * 100 < upper_len * self.options.size_ratio_percent {
                return Some(SimpleLeveledCompactionTask {
                    upper_level: Some(upper_idx + 1),
                    upper_level_sst_ids: snapshot.levels[upper_idx].1.clone(),
                    lower_level: upper_idx + 2,
                    lower_level_sst_ids: snapshot.levels[upper_idx + 1].1.clone(),
                    is_lower_level_bottom_level: upper_idx + 2 == self.options.max_levels,
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
    ///
    /// L0 files are removed BY ID (retain): files flushed while the compaction
    /// ran are not in the task's captured list and must survive. The lower
    /// level is replaced as a whole: simple leveled compaction always takes
    /// the ENTIRE level as input, so nothing can be inserted there concurrently
    /// (only compaction writes to levels >= 1). Returns the obsolete ids.
    pub fn apply_compaction_result(
        &self,
        snapshot: &LsmStorageState,
        task: &SimpleLeveledCompactionTask,
        output: &[usize],
    ) -> (LsmStorageState, Vec<usize>) {
        let mut snapshot = snapshot.clone();
        let mut obsolete = Vec::new();
        match task.upper_level {
            None => {
                // L0 -> L1: sanity-check the captured ids are all still present
                // (single-flight compaction + flush-only-adds guarantees this).
                for sst_id in &task.upper_level_sst_ids {
                    assert!(
                        snapshot.l0_sstables.contains(sst_id),
                        "captured L0 SST {sst_id} vanished before install"
                    );
                }
                snapshot
                    .l0_sstables
                    .retain(|sst_id| !task.upper_level_sst_ids.contains(sst_id));
                snapshot.levels[task.lower_level - 1].1 = output.to_vec();
            }
            Some(upper_level) => {
                // The upper level must be exactly the captured set: whole-level
                // input, nothing else may live there.
                assert_eq!(
                    snapshot.levels[upper_level - 1].1,
                    task.upper_level_sst_ids,
                    "upper level changed between task generation and install"
                );
                // The whole level was the input: it becomes empty; everything
                // merged into the output now lives in the lower level.
                snapshot.levels[upper_level - 1].1 = Vec::new();
                snapshot.levels[task.lower_level - 1].1 = output.to_vec();
            }
        }
        obsolete.extend(task.upper_level_sst_ids.iter().copied());
        obsolete.extend(task.lower_level_sst_ids.iter().copied());
        (snapshot, obsolete)
    }
}
