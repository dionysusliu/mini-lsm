# Simple Leveled Compaction

### Test Your Understanding

##### What is the estimated write amplification of leveled compaction?
For leveled LSM, write amp is roughly **one write per level (plus the initial flush)**

A standard approximation is: $WA \approx 1 + (L-1) \approx L$ , where $L$ is number of levels data passes through




##### What is the estimated read amplification of leveled compaction?
For point lookups in leveled compaction, a good estimate if $RA \approx l0 + (L-1)$, where $l0$ is number of tables in L0 level




##### Is it correct that a key will only be purged from the LSM tree if the user requests to delete it and it has been compacted in the bottom-most level?
It would also be purge if there are new version of that key in the database.




##### Is it a good strategy to periodically do a full compaction on the LSM tree? Why or why not?
- Usually not a good choice
- It causes a large one-time rewrite of most data, which **spikes**:
  - write amplification
  - disk bandwidth / CPU usage
  - latency jitter / possible write stalls
  - SSD wear

When full compaction can make sense:

- one-off maintenance window (offline / low-traffic)
- After massive deletes/TTL expiry to reclaim space quickly
- Before creating a snapshot artifact where compacted layout matters



##### Actively choosing some old files/levels to compact even if they do not violate the level amplifier would be a good choice, is it true? (Look at the Lethe paper!)

It is good for deletion/update-heavy workloads, and when you care about bounded data-retention/staleness. But it should be policy-drive (rated-limited, priority-based), not "compact old files aggressively all the time".



##### If the storage device can achieve a sustainable 1GB/s write throughput and the write amplification of the LSM tree is 10x, how much throughput can the user get from the LSM key-value interfaces?

only 1GB/s / 10 =100 MB/s.



##### Can you merge L1 and L3 directly if there are SST files in L2? Does it still produce correct result?

No, it breaks the versioning promise. We presume that upper layer carries new version of keys. merging L1 and L3 would leave some keys newer than L2 in L3. 



##### So far, we have assumed that our SST files use a monotonically increasing id as the file name. Is it okay to use <level>_<begin_key>_<end_key>.sst as the SST file name? What might be the potential problems with that? (You can ask yourself the same question in week 3...)

Nope, because we can't avoid file name collision. Consider the workload with begin key `1_1` and end key `2_2`, then the file name would be `1_1_2_2`. Now the key pair `{1_1_2, 2}` would require the same file name, leading to collisions.