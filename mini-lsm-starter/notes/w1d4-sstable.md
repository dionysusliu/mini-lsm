# SSTable

### Test your understanding

##### What is the time complexity of seeking a key in the SST?
`O(log(M) + log(N) + 2T)`

- `M`: # blocks in SSTable
- `N`: # entries per block
- `T`: time to read one block from the disk

We first do one binary search on BlockMeta vector to locate the candidate 
entry, which takes `O(log(M))` time; then we do one binary search on that 
entry, which takes `O(log(N) + T)`.
We might also need another disk I/O when no predecessor key is found on 
that entry, where we need to start from the next entry.


##### Where does the cursor stop when you seek a non-existent key in your implementation?
By design, it should stop at the first key >= itself.
```bash
[........ | x ....... ]
  < key     ^ >= key
```
- If such key exists in the sstable, it would stop at the smallest one.
- If such key doesn't exist (namely all keys in SSTable is smaller), then 
  the iterator builder would return an invalid iterator


##### Is it possible (or necessary) to do in-place updates of SST files?
SST files are intentionally immutable. We don't update SSTable on request,  
it's memtable's job.  Memtable always carries the freshest data.


##### An SST is usually large (i.e., 256MB). In this case, the cost of copying/expanding the Vec would be significant. Does your implementation allocate enough space for your SST builder in advance? How did you implement it?
Not now. A better implementation would call `Vec::with_capacity(...)`


##### Looking at the moka block cache, why does it return Arc<Error> instead of the original Error?
Because it could be shared among multiple threads, and `Arc` implements 
`Sync + Send` for safe cross-thread sharing.


##### Does the usage of a block cache guarantee that there will be at most a fixed number of blocks in memory? For example, if you have a moka block cache of 4GB and block size of 4KB, will there be more than 4GB/4KB number of blocks in memory at the same time?
No.
moka's capacity limit only applies to entries managed by the cache, and blocks can lives outside the cache
- evicted but still held by some iterator
- data in decode buffer or other allocations



##### Is it possible to store columnar data (i.e., a table of 100 integer columns) in an LSM engine? Is the current SST format still a good choice?
It is possible, but not ideal.

An LSM engine only requires data to be organized as sorted KV entries, so 
we can encode columnar entries' keys as (row_id, col_id) or store each 
column in a separate key range.

However, the current SST format is better suited for row-style workloads, 
wher eall fields of a record are stored together and fetched by primary keys.


##### Consider the case that the LSM engine is built on object store services (i.e., S3). How would you optimize/change the SST format/parameters and the block cache to make it suitable for such services?
The main tradeoff on S3/object storage is that request latency and request 
count matter much more than on local disk, so you want **fewer, larger 
remote reads**. It also means the SST format and cache strategy should be 
adapted to reduce small random reads.

First, SST files should be made much larger than in a local-disk setting, since
opening many small SSTs would cause too many object requests. Block size should
also be increased so that each range read fetches a larger chunk of useful data,
trading some read amplification for far fewer network round trips.

Second, the SST format should keep critical metadata compact and easy to read.
For example, indexes and bloom filters should be stored in well-known regions,
often near the end of the SST, so the engine can fetch metadata with one or a
small number of range requests before deciding which data blocks to read.

Third, reads should rely on S3 range requests to fetch only the needed parts of
an SST, but in larger units such as blocks or groups of blocks rather than tiny
pieces. It may also help to store a multi-level index so that locating a block
does not require loading the full index of a very large SST into memory.

Finally, the block cache should become much more aggressive. Besides caching
decoded blocks, it may be useful to cache raw downloaded byte ranges, index
blocks, and bloom filters, because avoiding a remote request is much more
important on object storage than avoiding local disk I/O. Prefetching nearby
blocks for scans is also beneficial.

So compared with local storage, an S3-friendly design would use larger SSTs,
larger blocks, compact fetchable metadata, range reads, and a much stronger
cache/prefetch strategy.

##### For now, we load the index of all SSTs into the memory. Assume you have a 16GB memory reserved for the indexes, can you estimate the maximum size of the database your LSM system can support? (That's why you need an index cache!)
Each block meta contains 4b of offset, at most 4KB of first key and last 
key, so with total size 8KB. A 16GB memory can hold roughly 2,000,000 such 
block meta structs. Each block would contain. 