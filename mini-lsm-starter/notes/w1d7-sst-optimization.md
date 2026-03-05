# SST Optimization

## Test your understanding

##### How does the bloom filter help with the SST filtering process? What kind of information can it tell you about a key? (may not exist/may exist/must exist/must not exist)
ensure zero false negative, namely it can tell a key "must not exist"

##### Consider the case that we need a backward iterator. Does our key compression affect backward iterators?
No, because each key is independently decodable.

##### Can you use bloom filters on scan?
That would be unnecessary because we can find the first key to scan in `O(log
(M+N))` of time, and doing bloom filter for each just add up unnecessary costs.


##### What might be the pros/cons of doing key-prefix encoding over adjacent keys instead of with the first key in the block?
Pros
    - better compression ratio (usually larger shared prefix with previous key)
    - smaller block bytes, potentially better cache/disk efficiency

Cons:
    - random access is worse: to decode key `i`, you may need to decode a 
chain from earliler keys
    - basically can't do binary search, and thus affect iterator seek

Pros of first key prefixing
    - Any entry can be decoded directly with one reference key
    - simpler decode and simpler binary search
    - good balance of small blocks too
    
