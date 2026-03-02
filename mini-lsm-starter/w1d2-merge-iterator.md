# Merge Iterators

## Test Your Understanding

#### What is the time/space complexity of using your merge iterator?
`O(N * log(M))`, where `N` is number of all keys in storage, and `M` is number of iterators / memtables

M is also the size of heap. We would traverse all keys in the storage engine, and for each key it takes:
- one pop operation -> `O(log(M))`
- one validity check -> `O(1)`, just reading the pointer metadata

So traversing all N keys takes `O(N * log(M))`


#### Why do we need a self-referential structure for memtable iterator?
It's not necessary, but having it saves a lot of time and complexity, without affecting program correctness.

The problem is about lifetime. Our core need is to make sure the generated iterators doesn't outlive the data they point to. 
So we need some methods to "encode" lifetime of SkipMap (in this case, equivalent to lifetime of data) so that we can use 
it in our iterator struct definitions. 

Having this self-referential structures enables us to simply tell compiler that "we need the iterator not outlive this SkipMap
iterator"


#### If a key is removed (there is a delete tombstone), do you need to return it to the user? Where did you handle this logic?
No we don't return to the user. We handled it at the user-level `LsmIterator`.

`LsmIterator` would shadow the delete key at `::new()` and `::next()`. The underlying `StorageIterator` would NOT skip them.


#### If a key has multiple versions, will the user see all of them? Where did you handle this logic?
No user would only see the latest version of key. We handle this logic at the `MergeIterator`

On each `::next()`, `MergeIterator` would keep checking whether current heap top has the "same" key as the current (the latest version). If so, it would just update this iterator and reinsert to heap, without emission.


#### If we want to get rid of self-referential structure and have a lifetime on the memtable iterator (i.e., MemtableIterator<'a>, where 'a = memtable or LsmStorageInner lifetime), is it still possible to implement the scan functionality?
Yes, it is possible, as long as we such lifetime is correctly enforced.

But this is VERY, VERY complex from the architectural aspect. let's see how it would look:

```rust
pub struct MemTableIterator<'a> {
    iter: SkipMapRangeIter<'a>,
    item: (Bytes, Bytes),
}
```

Here the lifetiem `'a` would be tied to the `SkipMap` the iterator borrows from, which in our case means that it's tied to the `Memtable` itself. To make it available to users, this lifetime would **propagate upwards** through every type containing the iterator:

```rust
MemTableIterator ->
MergeIterator ->
LsmIterator ->
FusedIterator
```

Now, let's consider the `scan` API on `LsmStorageInner`:

```rust
pub fn scan(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>) -> LsmIterator<'???> {
    let state = self.state.read(); // read lock guard
    // create iterators borrowing from state...
    // but state is dropped at end of this function!
}
```

1. scan` acquires a read lock on `state` 
2. and then it creates iterators borrowing that state
3. and then it **returns** this iterator, but **the lock guard is dropped, so it invalidates all the borrows**

To make this work you'd have to either hold the lock guard for the entire lifetime of the iterator (blocking all writers), or restructure the API so the caller manages the lock guard's lifetime explicitly — both are architecturally undesirable.

The ouroboros self-referential approarch **sidesteps** this entirely by having each `MemTableIterator` owns its `Arc<SkipMap>`. This keeps the map alive independently of the lock guard's lifetime.


#### What happens if (1) we create an iterator on the skiplist memtable (2) someone inserts new keys into the memtable (3) will the iterator see the new key?
I think this concurrency is handled by the crossbeam SkipMap. It depends on whether the underlying `SkipMapIter` would see the new key.


#### What happens if your key comparator cannot give the binary heap implementation a stable order?
The heap top would not necesarily be the latest key version, nor it might be the smallest key available.
This would break the presumption of `MergeIterator`: iterator with same keys are contiguous at the top of the heap. Our stale key detection algorithm won't work then.


#### Why do we need to ensure the merge iterator returns data in the iterator construction order?
This order dictates which key is fresher.


#### Is it possible to implement a Rust-style iterator (i.e., next(&self) -> (Key, Value)) for LSM iterators? What are the pros/cons?
A fundamental reason is **lifetime** problem. 

A Rust-style iterator returns a reference ` into itself:

```rust
fn next(&mut self) -> Option<(KeySlice<'_>, &'_ [u8])>
```

This works fine for in-memory iterators like `std::slice::Iter`. But for an LSM iterator that reads from disk, the returned reference would need to borrow from a buffer inside the iterator that holds the current page/block. The moment you call next() again, that buffer is overwritten — so the previous reference is invalidated. Rust's borrow checker would catch this: you cannot call next() while holding a reference to the previous result.

This means the caller is forced to clone every key and value before calling next() again, which defeats the purpose of returning references in the first place. The LSM iterator design sidesteps this by storing the current item in a buffer (item field) and exposing it via key() and value() as borrows from &self, while next() takes &mut self — the borrow checker then statically prevents calling next() while key() or value() references are still live.

Another design problem is about explicit error state management: we want `::is_valid()` be separated from `::next()`. Sometimes we just shouldn't call next(), like an IO error already occurred, and we don't want users to handle them. Having a validity check before moving also help user writes concurrent code via "checking before and after locking" pattern.


#### The scan interface is like fn scan(&self, lower: Bound<&[u8]>, upper: Bound<&[u8]>). How to make this API compatible with Rust-style range (i.e., key_a..key_b)? If you implement this, try to pass a full range .. to the interface and see what will happen.
Rust's range types (`a..b`, `a..=b`, `..`, etc.) all implement `RangeBounds<T>` trait from `std::ops`:
```rust
pub trait RangeBounds<T: ?Sized> {
    fn start_bound(&self) -> Bound<&T>;
    fn end_bound(&self) -> Bound<&T>;
}
```

So the way is to allow a new scan function take such trait-bound type:
```rust
pub fn scan_range(&self, range: impl RangeBounds<&[u8]>) -> LsmIterator {
    self.scan_inner(
        range.start_bound().map(|b| b.as_ref()), 
        range.end_bound().map(|b| b.as_ref()),
    )
}
```


#### The starter code provides the merge iterator interface to store Box<I> instead of I. What might be the reason behind that?
1. Trait object compatibility
The merge iterator needs to store heterogeneous iterator types in the future -- memtable, SSTable, concat, etc, all implementing `StorageIterator`. Without `Box`, the BinaryHeap must be homogeneous. With `Box<dyn StorageIterator>`, we can **mix** different iterator types in the same heap. 

2. Stack size control
Iterators can be large structs:
    - `MemtableIterators` contains a ouroboros wrapper
    - `SSTableIterators` will contain block buffers, file handle, ...
Storing them directly in `HeapWrapper` would make HeapWrapper itself large, and also make BinaryHeap operations memory-costly.

3. Recursive iterator types

`MergeIterator<I>` itself implements `StorageIterator`. 

We can have `MergeIterator<MergeIterator<I>>` for **multi-level merging**. 

Without `Box`, this nesting would create **infinitely sized types**, and compiler won't resolve. `Box` breaks the recursion by introducing a layer of indirection with a known size.
