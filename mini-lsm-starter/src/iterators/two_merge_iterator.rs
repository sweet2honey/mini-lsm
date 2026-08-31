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

use anyhow::Result;

use super::StorageIterator;

/// Merges two iterators of different types into one. If the two iterators have the same key, only
/// produce the key once and prefer the entry from A.
pub struct TwoMergeIterator<A: StorageIterator, B: StorageIterator> {
    a: A,
    b: B,
    /// Whether the merged cursor currently reads from A (the newer, tie-winning side).
    use_a: bool,
}

impl<
    A: 'static + StorageIterator,
    B: 'static + for<'a> StorageIterator<KeyType<'a> = A::KeyType<'a>>,
> TwoMergeIterator<A, B>
{
    pub fn create(a: A, b: B) -> Result<Self> {
        let mut iter = Self { a, b, use_a: false };
        iter.recompute_leader();
        Ok(iter)
    }

    /// A (newer) leads while it is valid and its key is <= B's; ties go to A. When only
    /// one side is valid, that side leads.
    fn recompute_leader(&mut self) {
        self.use_a = if !self.a.is_valid() {
            false
        } else if !self.b.is_valid() {
            true
        } else {
            self.a.key() <= self.b.key()
        };
    }
}

impl<
    A: 'static + StorageIterator,
    B: 'static + for<'a> StorageIterator<KeyType<'a> = A::KeyType<'a>>,
> StorageIterator for TwoMergeIterator<A, B>
{
    type KeyType<'a> = A::KeyType<'a>;

    fn key(&self) -> Self::KeyType<'_> {
        if self.use_a {
            self.a.key()
        } else {
            self.b.key()
        }
    }

    fn value(&self) -> &[u8] {
        if self.use_a {
            self.a.value()
        } else {
            self.b.value()
        }
    }

    fn is_valid(&self) -> bool {
        if self.use_a {
            self.a.is_valid()
        } else {
            self.b.is_valid()
        }
    }

    fn num_active_iterators(&self) -> usize {
        self.a.num_active_iterators() + self.b.num_active_iterators()
    }

    fn next(&mut self) -> Result<()> {
        if !self.is_valid() {
            return Ok(());
        }
        if self.use_a {
            // Tie drain: B sits on the same key A is about to emit. B must advance past
            // its stale copy too, or it resurfaces as the merged head once A moves on.
            let drain_b = self.b.is_valid() && self.a.key() == self.b.key();
            self.a.next()?;
            if drain_b {
                self.b.next()?;
            }
        } else {
            self.b.next()?;
        }
        self.recompute_leader();
        Ok(())
    }
}
