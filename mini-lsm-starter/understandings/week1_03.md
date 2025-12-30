## Test Your Understanding

* What is the time complexity of seeking a key in the block?
    `seek_to_key` is `O(log n)` over `offsets`. Each comparison takes `O(L)`(key length).
    Thus `O(log n * L)` in total.

* Where does the cursor stop when you seek a non-existent key in your implementation?
    The first key bigger than search key.

* So `Block` is simply a vector of raw data and a vector of offsets. Can we change them to `Byte` and `Arc<[u16]>`, and change all the iterator interfaces to return `Byte` instead of `&[u8]`? (Assume that we use `Byte::slice` to return a slice of the block without copying.) What are the pros/cons?
    Pros: Zero-copy `Bytes::slice` type.
    Cons: Ref-counting has overhead. `Arc[u16]` is immutable, can not grow.
    
    `&[u8]` is ensured to be safe by the compiler and has best performance.
    Use of `Byte` has both sides of the coin.

* What is the endian of the numbers written into the blocks in your implementation?
    Big endian.

* Is your implementation prune to a maliciously-built block? Will there be invalid memory access, or OOMs, if a user deliberately construct an invalid block?
    The `num_of_entry` could be wrong, causing large vector allocation

* Can a block contain duplicated keys?
    Yes. The builder does not verify the input during `add` is called. Is just assumes the user offer data in sorted order.

* What happens if the user adds a key larger than the target block size?
    A big entry is allowed, and it would take up a new block, see `BlockBuildad::add`.

* Consider the case that the LSM engine is built on object store services (S3). How would you optimize/change the block format and parameters to make it suitable for such services?
    - Use meta data sections to help index blocks.
    - Use larger blocks.
    - Compress block, use bloom filters.
    - Fit storage service size&price.

* Do you love bubble tea? Why or why not?
    Can't love it any more.