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

// #![allow(unused_variables)] // TODO(you): remove this lint after implementing this mod
// #![allow(dead_code)] // TODO(you): remove this lint after implementing this mod

use std::cmp::{self};
use std::collections::BinaryHeap;
use std::collections::binary_heap::PeekMut;

use anyhow::Result;

use crate::key::KeySlice;

use super::StorageIterator;

struct HeapWrapper<I: StorageIterator>(pub usize, pub Box<I>);

impl<I: StorageIterator> PartialEq for HeapWrapper<I> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == cmp::Ordering::Equal
    }
}

impl<I: StorageIterator> Eq for HeapWrapper<I> {}

impl<I: StorageIterator> PartialOrd for HeapWrapper<I> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<I: StorageIterator> Ord for HeapWrapper<I> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        // This would yield the 'smallest' key
        self.1
            .key()
            .cmp(&other.1.key()) // Compare keys
            .then(self.0.cmp(&other.0)) // Break ties with index
            .reverse() // REVERSE the result!
    }
}

/// Merge multiple iterators of the same type. If the same key occurs multiple times in some
/// iterators, prefer the one with smaller index.
pub struct MergeIterator<I: StorageIterator> {
    iters: BinaryHeap<HeapWrapper<I>>,
    current: Option<HeapWrapper<I>>,
}

impl<I: StorageIterator> MergeIterator<I> {
    pub fn create(iters: Vec<Box<I>>) -> Self {
        if iters.is_empty() {
            return Self {
                iters: BinaryHeap::new(),
                current: None,
            };
        }

        //* Turn Box<I>(or say Box<StorageIterator>) into `HeapWrapper`s
        // You may create a `BinaryHeap` and push back one by one
        // We use `BinaryHeap::from(vec: Vec<T, A>)` here
        let wrappers = iters
            .into_iter()
            .enumerate()
            .filter_map(|(idx, iter)| {
                if iter.is_valid() {
                    // Turn valid iter into `HeapWrapper`
                    Some(HeapWrapper(idx, iter))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let mut heap = BinaryHeap::from(wrappers);
        let current = heap.pop();

        Self {
            iters: heap,
            current: current,
        }
    }
}

impl<I: 'static + for<'a> StorageIterator<KeyType<'a> = KeySlice<'a>>> StorageIterator
    for MergeIterator<I>
{
    type KeyType<'a> = KeySlice<'a>;

    fn key(&self) -> KeySlice<'_> {
        //* Access validation is ensured by `is_valid` outside.
        self.current.as_ref().unwrap().1.key()
    }

    fn value(&self) -> &[u8] {
        //* Access validation is ensured by `is_valid` outside.
        self.current.as_ref().unwrap().1.value()
    }

    fn is_valid(&self) -> bool {
        self.current
            .as_ref() // `as_ref` is needed to avoid lambda consuming it
            .is_some_and(|wrapper| wrapper.1.is_valid())
    }

    //* We have N(`iters.len()`) + 1(`current`) iterators which yields data.
    //* The core idea is to advance to next `smallest` element among them,
    //* `BinaryHeap` helps managing N of them, so we mainly do `current` and `heap.peek()` stuff.
    // 1. Skip duplicates in other iterators: While heap top has the same key as current, advance those iterators and remove if they become invalid.
    // 2. Advance current iterator: Call next() on the current iterator.
    // 3. Handle invalid current: If current becomes invalid, replace it with the next item from the heap (or set to None if heap is empty).
    // 4. Maintain heap property: If current is still valid, compare with heap top. If current is now larger, swap them to maintain the invariant that current has the smallest key.
    fn next(&mut self) -> Result<()> {
        let current_key = self.current.as_ref().unwrap().1.key();

        //* 1. Skip all iterators that have same key as current
        while let Some(mut heap) = self.iters.peek_mut() {
            let storage = &mut heap.1;

            if storage.key() == current_key {
                // advance the storage iter, if failed, remove it
                if let e @ Err(_) = storage.next() {
                    PeekMut::pop(heap);
                    return e;
                }

                // if storage is not valid(no need to traverse anymore), remove it
                if !storage.is_valid() {
                    PeekMut::pop(heap);
                }
            } else {
                // Means next key from the heap is different from current
                break;
            }
        }

        //* 2. Advance current iterator
        let current_wrapper = self.current.as_mut().unwrap();
        current_wrapper.1.next()?;

        // The iter may be invalid
        //* 3. If current is invalid, pop from heap
        if !current_wrapper.1.is_valid() {
            if let Some(new_current) = self.iters.pop() {
                *current_wrapper = new_current;
            }
            return Ok(());
        }

        // The key may be ordered after the heap top
        //* 4. Compare with heap top, putting the corroctly sorted value into `self.current`
        if let Some(mut heap_top) = self.iters.peek_mut() {
            // The greater one should be presented
            // `greater` means `Ord::Greater`, indicating smaller key or smaller index
            if *current_wrapper < *heap_top {
                std::mem::swap(current_wrapper, &mut *heap_top);
            }
        }

        Ok(())
    }
}
