# Full Compaction

### Test your understanding

##### What are the definitions of read/write/space amplifications? (This is covered in the overview chapter)
1. read amplification: the number of memtable & sstable to read for one read request
2. write amplification: total bytes written to disk per byte of user data written
3. space amplification: on-disk bytes used by DB divided by live user-data bytes


##### What are the ways to accurately compute the read/write/space amplifications, and what are the ways to estimate them?
**Accurate**:
    - read amp: instrument actual reads per query (#SST checked, #block 
read, bytes read, IO ops), then aggregate percentiles
    - Write amp: track cumulative bytes written to disk (WAL + flush + 
compaction outputs) divided by cumulative user bytes ingested.
    - Space amp: measure live on-disk file bytes at time t divided by 
logical live dataset bytes at t.

**Estimate**
    - Read Amp: `RA = #L0_tables + #non_empty_level`
        - assume may touch all l0 files plus one run per non-empty level
    - Write Amp: `WA = total_writes / total_flushes`
        - `total_flushes += 1` for each new flushed SST
        - `total_writes += 1` for each SST written (flush output + compaction output SSTs)
    - Maximum space usage: `max_space / total_flushes`
        - `max_space = max(max_space, file_list.len())`


##### Is it correct that a key will take some storage space even if a user requests to delete it?
Yes, because deletion would remain tombstone when they first flushed to the disk


##### Given that compaction takes a lot of write bandwidth and read bandwidth and may interfere with foreground operations, it is a good idea to postpone compaction when there are large write flow. It is even beneficial to stop/pause existing compaction tasks in this situation. What do you think of this idea? (Read the SILK: Preventing Latency Spikes in Log-Structured Merge Key-Value Stores paper!)
Pausing/postponing compaction during heavy foreground load can reduce tail 
latency, but if overdone it causes L0 growth, higher read amp, worse space 
amp, and eventually stalls. So compaction should be workload-aware and 
rate-limited, not simply "off under high load.". 

SILK-style takeaway: schedule compaction with latency-aware control loop, 
adaptive throttling, and bounded debt/backlog so foreground latency and 
long-term health are both maintained.


##### Is it a good idea to use/fill the block cache for compactions? Or is it better to fully bypass the block cache when compaction?
Using block cache makes sense, since sstable are immutable and it saves 
time to do disk IO. Filling block cache hurts performance, because the 
compaction load each block only once and it can pollute the cache. However, 
for sequential read we  tend to prefetch some adjacent blocks together. In 
this case, the block cache doesn't help much  and could pollute the cache.


##### Does it make sense to have a struct ConcatIterator<I: StorageIterator> in the system?
No, because concat iterator are specifically used to wrap the SStable 
iterator. 


##### Some researchers/engineers propose to offload compaction to a remote server or a serverless lambda function. What are the benefits, and what might be the potential challenges and performance impacts of doing remote compaction? (Think of the point when a compaction completes and what happens to the block cache on the next read request...)
- Benefits
  1. Offloads CPU, disk bandwidth, and IO contention from foreground node.
  2. Better elasticity and cost control (scale compaction wokers independently)
  3. Can smooth p99 latency on serving node during heavy compaction periods

- Impacts
  1. Data transfer overhead (shipping SSTs/metadata) can dominate cost and 
     latency
  2. Coordination complexity: snapshots, versioning, atomic install of 
     compaction result, failure recovery
  3. Higher compaction completion latency and staleness window
  4. Cache disruption: newly compacted SSTs have new file/block ids, so 
     block cache on serving node sees cold misses after install