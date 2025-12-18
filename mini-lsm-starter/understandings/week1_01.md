## Test Your Understanding

* Why doesn't the memtable provide a `delete` API?
    `delete` 通过写入 tombstone 实现。
    - 对 WAL 日志友好，因为都是追加；
    - flush、压缩的时候不需要考虑具体操作的类型，简单；

* Does it make sense for the memtable to store all write operations instead of only the latest version of a key? For example, the user puts a->1, a->2, and a->3 into the same memtable.
    对于一个 memtable 来说，保留最新的值就足够了。
    - 查找、flush 简单
    - 省空间

* Is it possible to use other data structures as the memtable in LSM? What are the pros/cons of using the skiplist?
    任何 map，甚至是 ordered vector 都可以。
    crossbeam skiplist 是无锁的；但是指针的缓存友好性可能差一些。

* Why do we need a combination of `state` and `state_lock`? Can we only use `state.read()` and `state.write()`?
    `state` 是 `RwLock`，允许快速地拷贝 snapshot 读取，缩短 COW 持有锁的时间。
    `state_lock` 用于统筹“获取 snapshot -> IO 操作 -> 交换状态”的流程。

* Why does the order to store and to probe the memtables matter? If a key appears in multiple memtables, which version should you return to the user?
    LSM’s “latest update wins” semantics.

* Is the memory layout of the memtable efficient / does it have good data locality? (Think of how `Byte` is implemented and stored in the skiplist...) What are the possible optimizations to make the memtable more efficient?
    `Bytes` 是 reference-counted 的堆上小对象，对缓存不友好。
    优化方式可以有：
    - arena-allocating
    - prefix compression
    - small object inline buffer

* So we are using `parking_lot` locks in this course. Is its read-write lock a fair lock? What might happen to the readers trying to acquire the lock if there is one writer waiting for existing readers to stop?
    > This lock uses a task-fair locking policy which avoids both reader and writer starvation. This means that readers trying to acquire the lock will block even if the lock is unlocked when there are writers waiting to acquire the lock. Because of this, attempts to recursively acquire a read lock within a single thread may result in a deadlock.

* After freezing the memtable, is it possible that some threads still hold the old LSM state and wrote into these immutable memtables? How does your solution prevent it from happening?
    `put` 需要读锁，而 `freeze` 需要写锁，那么不会发生写入 immutable 的情况。


* There are several places that you might first acquire a read lock on state, then drop it and acquire a write lock (these two operations might be in different functions but they happened sequentially due to one function calls the other). How does it differ from directly upgrading the read lock to a write lock? Is it necessary to upgrade instead of acquiring and dropping and what is the cost of doing the upgrade?
    放掉再获取锁可能有并发窗口，需要 double-check 条件。
    读锁升级可能更重型，而且相当于多占用了读取的时间。