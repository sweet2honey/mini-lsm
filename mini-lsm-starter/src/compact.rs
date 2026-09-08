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

#![allow(unused_variables)] // TODO(you): remove this lint after implementing this mod
#![allow(dead_code)] // TODO(you): remove this lint after implementing this mod

mod leveled;
mod simple_leveled;
mod tiered;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
pub use leveled::{LeveledCompactionController, LeveledCompactionOptions, LeveledCompactionTask};
use serde::{Deserialize, Serialize};
pub use simple_leveled::{
    SimpleLeveledCompactionController, SimpleLeveledCompactionOptions, SimpleLeveledCompactionTask,
};
pub use tiered::{TieredCompactionController, TieredCompactionOptions, TieredCompactionTask};

use crate::iterators::StorageIterator;
use crate::iterators::merge_iterator::MergeIterator;
use crate::lsm_storage::{LsmStorageInner, LsmStorageState};
use crate::table::{SsTable, SsTableBuilder, SsTableIterator};

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
    /// Merge the SSTs named by the task into a new sorted run. Runs without
    /// `state_lock`: the inputs are immutable files and the outputs stay
    /// invisible until `force_full_compaction` installs them.
    fn compact(&self, task: &CompactionTask) -> Result<Vec<Arc<SsTable>>> {
        let CompactionTask::ForceFullCompaction {
            l0_sstables,
            l1_sstables,
        } = task
        else {
            // The Simple/Tiered/Leveled strategies arrive in Week 2 Days 2-4.
            unimplemented!("only ForceFullCompaction is implemented so far")
        };

        // Children run newest-first so the merge's index tie-break keeps the
        // newest version of each key: the L0 list is already stored
        // newest->oldest, and the L1 SSTs follow (their ranges do not
        // overlap, so only an L0-vs-L1 tie is possible, and L0 wins it).
        let snapshot = self.state.read().clone();
        let mut iters: Vec<Box<SsTableIterator>> =
            Vec::with_capacity(l0_sstables.len() + l1_sstables.len());
        for sst_id in l0_sstables.iter().chain(l1_sstables.iter()) {
            let sst = snapshot
                .sstables
                .get(sst_id)
                .expect("compaction task references a missing SST")
                .clone();
            iters.push(Box::new(SsTableIterator::create_and_seek_to_first(sst)?));
        }
        let mut iter = MergeIterator::create(iters);

        // The merge drains every child at the emitted key, so each key reaches
        // this loop exactly once, already resolved to its newest version. A
        // tombstone winner is omitted outright: this task includes every
        // older version, so nothing below can resurface. When tombstones are
        // the only survivors, no SST is produced at all.
        let mut ssts: Vec<Arc<SsTable>> = Vec::new();
        let mut builder = SsTableBuilder::new(self.options.block_size);
        let mut builder_entries = 0;
        while iter.is_valid() {
            if !iter.value().is_empty() {
                builder.add(iter.key(), iter.value());
                builder_entries += 1;
                // Soft size limit, like the memtable freeze check: only the
                // last output SST may end up smaller than target_sst_size.
                if builder.estimated_size() >= self.options.target_sst_size {
                    let sst_id = self.next_sst_id();
                    ssts.push(Arc::new(builder.build(
                        sst_id,
                        Some(self.block_cache.clone()),
                        self.path_of_sst(sst_id),
                    )?));
                    builder = SsTableBuilder::new(self.options.block_size);
                    builder_entries = 0;
                }
            }
            iter.next()?;
        }
        if builder_entries > 0 {
            let sst_id = self.next_sst_id();
            ssts.push(Arc::new(builder.build(
                sst_id,
                Some(self.block_cache.clone()),
                self.path_of_sst(sst_id),
            )?));
        }
        Ok(ssts)
    }

    /// Compact every SST: capture a task, merge and write the outputs with no
    /// lock held, then install under `state_lock` and delete the inputs last.
    pub fn force_full_compaction(&self) -> Result<()> {
        // The captured ids are the authority for what this task owns. An SST
        // flushed after this moment is not in the list and must survive the
        // install.
        let (l0_to_compact, l1_to_compact) = {
            let snapshot = self.state.read().clone();
            (snapshot.l0_sstables.clone(), snapshot.levels[0].1.clone())
        };
        let task = CompactionTask::ForceFullCompaction {
            l0_sstables: l0_to_compact.clone(),
            l1_sstables: l1_to_compact.clone(),
        };

        let new_ssts = self.compact(&task)?;

        // Install: one snapshot swap under state_lock. L0 removal is by id
        // from the captured list — never a rebuild — so anything flushed
        // during the merge stays in L0.
        {
            let _state_lock = self.state_lock.lock();
            let mut guard = self.state.write();
            let mut snapshot = guard.as_ref().clone();
            snapshot
                .l0_sstables
                .retain(|sst_id| !l0_to_compact.contains(sst_id));
            snapshot.levels[0].1 = new_ssts.iter().map(|sst| sst.sst_id()).collect();
            for sst_id in l0_to_compact.iter().chain(l1_to_compact.iter()) {
                snapshot.sstables.remove(sst_id);
            }
            for sst in new_ssts.iter() {
                snapshot.sstables.insert(sst.sst_id(), sst.clone());
            }
            *guard = Arc::new(snapshot);
        }

        // Delete after install — the commit point. Readers on older snapshots
        // hold Arcs with open fds; Unix keeps an unlinked inode readable
        // until its last handle closes, so they finish undisturbed.
        for sst_id in l0_to_compact.iter().chain(l1_to_compact.iter()) {
            std::fs::remove_file(self.path_of_sst(*sst_id))?;
        }
        Ok(())
    }

    fn trigger_compaction(&self) -> Result<()> {
        unimplemented!()
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
        // Drain one oldest immutable memtable per wake-up; a queue that outgrows the
        // limit stays visible as pressure rather than being hidden in a long flush.
        let imm_memtable_count = self.state.read().imm_memtables.len();
        if imm_memtable_count >= self.options.num_memtable_limit {
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
