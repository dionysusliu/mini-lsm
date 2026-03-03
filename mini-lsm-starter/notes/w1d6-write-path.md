# Write Path

### Test Your Understanding

##### What happens if a user requests to delete a key twice?
In an LSM tree, delete is usually represented by writing a tombstone entry 
for the key. Therefore, deleting the same key twice will simply create 
another tombstone for that key.

Functionally, nothing new happens after first delete: the key is still 
considered deleted. However, the second delete still adds another record to 
the write path, consume space, and may increase write amplification until 
compaction removes obsolete versions and redundant tombstones.


##### How much memory (or number of blocks) will be loaded into memory at the same time when the iterator is initialized?
One block per SST iterator involved in the merge, because each iterator 
needs to seek to its starting position when the storage iterator is 
initialized.


##### Some crazy users want to fork their LSM tree. They want to start the engine to ingest some data, and then fork it, so that they get two identical dataset and then operate on them separately. An easy but not efficient way to implement is to simply copy all SSTs and the in-memory structures to a new directory and start the engine. However, note that we never modify the on-disk files, and we can actually reuse the SST files from the parent engine. How do you think you can implement this fork functionality efficiently without copying data? (Check out Neon Branching).
A more efficient implementation is to treat SST files as shared immutable 
base data and only copy metadata/state, not the data file themselves.

When the engine is forker, the child can inherit the same set of existing 
SST files from the parent, since SSTs are immutable and safe to share. The 
parent and child would each get their own metadata/manifest state 
describing which SSTs belong to their branch.

In this design, the fork is implemented as copy-on-write at the metadata level:
both branches initially reference the same SST files, and divergence 
happens only for new writes and future compaction outputs. Old SSTs can be 
reference-counted or garbage collected only after no branch still points to 
them.


##### Imagine you are building a multi-tenant LSM system where you host 10k databases on a single 128GB memory machine. The memtable size limit is set to 256MB. How much memory for memtable do you need for this setup?
Assume no-sharing. Then it requires 10k * 256MB = 2.56TB 


##### Obviously, you don't have enough memory for all these memtables. Assume each user still has their own memtable, how can you design the memtable flush policy to make it work? Does it make sense to make all these users share the same memtable (i.e., by encoding a tenant ID as the key prefix)?
Since it is impossible to reserve a large memtable for every tenant, the 
system should use a global memory budget rather than a fixed per-tenant 
memory reservation.

Each tenant can still logically have its own memtable, but memtable flush 
should be triggered by global memory pressure. For example, when total 
memtable memory across all tenants exceeds a threshold, the system can 
choose some tenant memtables to flush, such as:
    - the largest memtable
    - the coldest memtable  
    - the oldest immutable memtable
    - or based on fairness across tenants

This allows active tenants to use more memory temporarily, while inactive 
tenants consume little or no memory.

Sharing one physical memtable across all tenants by encoding tenant ID as 
as key prefix is possible, and it is usually not a good default design. 
Although it may improve memory utilization, it mixes tenants together in 
the same write path and flush path, making isolation, fairness, per-tenant 
rate limiting, compaction, and recovery more complicated. A hot tenant 
could also interfere with cold tenants more easily.

So a better design is usually:
    - separate logical memtables per tenant,
    - but a shared global memory budget and a global flush scheduler.