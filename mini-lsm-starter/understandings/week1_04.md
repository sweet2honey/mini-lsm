## Test Your Understanding

* What is the time complexity of seeking a key in the SST?
    `find_block_idx` => `O(log Block)`
    `BlockIterator::seek_to_key` => `O(log Entry)`
    In total: `O(log Block + log Entry)`

* Where does the cursor stop when you seek a non-existent key in your implementation?
    Either:
    - Inside the sst, on the first key greater than `search key`
    
    Or:
    - Outside the sst, end of the sst. If the `search key` is greater than last key of the last block.

* Is it possible (or necessary) to do in-place updates of SST files?
    Almost never do it. SST are designed to be immutable and sorted.

* An SST is usually large (i.e., 256MB). In this case, the cost of copying/expanding the `Vec` would be significant. Does your implementation allocate enough space for your SST builder in advance? How did you implement it?
    Yes to some extend, the code reserves estimated size instead of 256MB.

* Looking at the `moka` block cache, why does it return `Arc<Error>` instead of the original `Error`?
    Error could be shared, and it fits Moka's API(Send + Sync).

* Does the usage of a block cache guarantee that there will be at most a fixed number of blocks in memory? For example, if you have a `moka` block cache of 4GB and block size of 4KB, will there be more than 4GB/4KB number of blocks in memory at the same time?
    No, it only controls the max entries within the cache.
    Using `Arc` could make data outlive the cache even if the entry is supposed to be evicted.

* Is it possible to store columnar data (i.e., a table of 100 integer columns) in an LSM engine? Is the current SST format still a good choice?
    It is fine but not optimal. It is better to change the SST layout, e.g. split columns, add indexes.

* Consider the case that the LSM engine is built on object store services (i.e., S3). How would you optimize/change the SST format/parameters and the block cache to make it suitable for such services?
    Same as the way for blocks. Can use some local disk cache as well.

* For now, we load the index of all SSTs into the memory. Assume you have a 16GB memory reserved for the indexes, can you estimate the maximum size of the database your LSM system can support? (That's why you need an index cache!)
    Metadata of a block uses `offset + 2 * key` in size. Say it takes up 64 bytes.
    If index takes up 1% of data, 16 GB of index can map to 1.6 TB of data.