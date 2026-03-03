# Read Path

### Test Your Understanding

##### Consider the case that a user has an iterator that iterates the whole storage engine, and the storage engine is 1TB large, so that it takes ~1 hour to scan all the data. What would be the problems if the user does so? (This is a good question and we will ask it several times at different points of the course...)
1. pollutes cache. Very low cache hit rate, because each block is accessed 
   only once. 
2. consumes substantial disk bandwidth, can interfere with foreground reads 
   and writes
3. storage engine needs to hold the consistent snapshot. This delays 
   compaction and GC, because old memtables, SST, or versions must stay 
   alive until the scan finishes


##### Another popular interface provided by some LSM-tree storage engines is multi-get (or vectored get). The user can pass a list of keys that they want to retrieve. The interface returns the value of each of the key. For example, multi_get(vec!["a", "b", "c", "d"]) -> a=1,b=2,c=3,d=4. Obviously, an easy implementation is to simply doing a single get for each of the key. How will you implement the multi-get interface, and what optimizations you can do to make it more efficient? (Hint: some operations during the get process will only need to be done once for all keys, and besides that, you can think of an improved disk I/O interface to better support this multi-get interface).
A basic implementation is to process all requested keys under one storage-state
snapshot instead of taking a new snapshot for each individual `get`. Then, for
each key, check the memtable, immutable memtables, and SSTs in the usual order.

A more efficient implementation should share work across keys.

First, sort or group the requested keys. Then for each SST, use its key range
(and later bloom filter) to determine which of the requested keys may exist in
that SST, instead of probing every SST for every key.

Second, if multiple keys map to the same data block, read that block only once
and answer all of those keys together. This avoids repeated index lookups and
repeated block reads.

Third, metadata work such as snapshot acquisition, SST selection, bloom filter
checks, and index lookups can be done once per batch rather than once per key.

Finally, the disk I/O interface can be improved by supporting vectored or batched
reads. If several required blocks are close in the same SST, the engine can issue
a single larger read or a small number of range reads instead of many tiny random
reads. On systems that support async I/O, these block reads can also be submitted
in parallel.

So an efficient multi-get should:
1. take one consistent snapshot for the whole batch,
2. group keys by SST and by data block,
3. read each needed block at most once,
4. batch or parallelize disk reads where possible.