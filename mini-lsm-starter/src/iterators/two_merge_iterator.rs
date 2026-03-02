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

use anyhow::Result;

use super::StorageIterator;

/// Merges two iterators of different types into one. If the two iterators have the same key, only
/// produce the key once and prefer the entry from A.
pub struct TwoMergeIterator<A: StorageIterator, B: StorageIterator> {
    a: A,
    b: B,
    // Add fields as need
    use_a: bool, // true if next key is from a, else next key is from b
}

impl<
    A: 'static + StorageIterator,
    B: 'static + for<'a> StorageIterator<KeyType<'a> = A::KeyType<'a>>,
> TwoMergeIterator<A, B>
{
    pub fn create(a: A, mut b: B) -> Result<Self> {
        if a.is_valid() && b.is_valid() && a.key() == b.key() {
            b.next()?;
        }
        let use_a = Self::choose_a(&a, &b);
        Ok(Self { a, b, use_a })
    }

    fn choose_a(a: &A, b: &B) -> bool {
        match (a.is_valid(), b.is_valid()) {
            (true, true) => a.key() <= b.key(),
            (true, false) => true,
            (false, true) => false,
            (false, false) => false,
        }
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
        self.a.is_valid() || self.b.is_valid()
    }

    fn next(&mut self) -> Result<()> {
        // make progress
        if self.use_a {
            let skip_b = self.a.is_valid() && self.b.is_valid() && self.a.key() == self.b.key();
            self.a.next()?;
            if skip_b {
                self.b.next()?;
            }
        } else {
            let skip_a = self.a.is_valid() && self.b.is_valid() && self.a.key() == self.b.key();
            self.b.next()?;
            if skip_a {
                self.a.next()?;
            }
        }

        // update next key to use
        self.use_a = Self::choose_a(&self.a, &self.b);

        Ok(())
    }
}
