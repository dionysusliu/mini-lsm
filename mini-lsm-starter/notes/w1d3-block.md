# Block

### Test your understanding

#### What is the time complexity of seeking a key in the block?
`O(log(N))`, where N is number of keys in the block. We applied binary search.


#### Where does the cursor stop when you seek a non-existent key in your implementation?
In our implementation, `left` stops when `left == offsets.len()`, namely number of elements


#### So Block is simply a vector of raw data and a vector of offsets. Can we change them to Byte and Arc<[u16]>, and change all the iterator interfaces to return Byte instead of &[u8]? (Assume that we use Byte::slice to return a slice of the block without copying.) What are the pros/cons?
**pros of `Bytes`/`Arc[u16]`**
- `Bytes::slice()` returns a zero-copy sub-view of same buffer. So returning `Bytes` from `value()` instead of `&[u8]` means callers can hold onto the value indenpendently of the iterators' lifetime. In this case, caller no longer need to clone &[u8] into owned data if they want to store the value.
- `Arc[u16]` allows multiple iterators share the offset table withotu copying. This is more efficient when there are many concurrent readers

**cons**
- `Bytes` is a fat pointer, so heavier than plain `&[u8]`. For hot path where `value()` is called millions of times per second, the refcount increments on every `Bytes::slice()` are costly.
- `Arc<[u16]>` saves no time when there is only one reader

**vector is good for builder**
`Vec` gives you a growable buffer that's natural during block building
    - you `push` and `put` bytes **incrementally**
`
Once building is done and the block is frozen, `Vec`'s growability is wasted. Converting to `Bytes` and `Arc<[u16]> at `build()` time could:
    - makes the **immutability** explicit 
    - enables zero-copy sharing during the **reading** phase

So the ideal design is: use `Vec` during *construction*, convert to `Bytes/Arc` at the **boundary between write path and read path**.


#### What is the endian of the numbers written into the blocks in your implementation?
Big endian. Because `BufMut::put_` family uses **big endian** by default.


#### Is your implementation prune to a maliciously-built block? Will there be invalid memory access, or OOMs, if a user deliberately construct an invalid block?
No, it doesn't check the actual format of block for now. There is no guarantee that the user could create their own SSTable block and trick the program.


#### Can a block contain duplicated keys?
yes, the block can contain duplicated keys. There are no restrictions on this.

And this should not be enforced at the "block layer". The block is just a dumb storage primitive -- it just encodes and decodes raw key-value bytes. 

Uniqueness is a **semantic invariant** that belongs to a higher layer. The block has no knowledge of whether it's storing memtable data, SST data, or compaction output. 


#### What happens if the user adds a key larger than the target block size?
It would sure create a block larger than target block size, because we do not do size thresholding for the first key.

This is intentional by design: the system for now must take keys. So rejecting the first entry would cause the SSTable builder to loop forever, repeatly trying to write the same oversized entry to a new block.

**Production handling**
Production systems like RocksDB handles this by:
    - compressing large value and store inline
    - using a separate **blob file**, and SST only stores a pointer

#### Consider the case that the LSM engine is built on object store services (S3). How would you optimize/change the block format and parameters to make it suitable for such services?

**Block Size**
4KB blocks are optimized for local SSD when random I/O is fast. 

On S3, each GetObject request has 10~100ms of fixed latency regardless of how much data we read. So we should *increase block size* dramatically - 512KB to 1MB - to amortize the per-key latency over more data. The marginal cost of reading more bytes is low once the request is already in flight.

**SST Size**
Similarly, SSTs should be much larger -- potentialy gigabytes -- to reduce the number of objects in S3 and the number of API calls during compaction.

**Catalog**
The catalog (manifest) tracks which key live in which S3 object at which byte range. Now, we can use S3's `Range` header to fetch only the **relevant block** within a large object, without downloading the entire SST. 

This is critical -- without range reads, you'd have to download entire multi-GB SSTs for point lookups.

**Write path**
The WAL needs to be local, since S3 is unsuitable for the low-latency sequential writes WAL requires. Some systems use a two-tier approach: local NVMe for the write path, S3 for the cold read path.


