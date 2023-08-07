# Compression Blocks

## Written in random order (as they are processed)

* Primarily handled by BytesBlockStore
* Submitted to the compression thread pool as each block fills up
* Submitted in Compression Packet, with AtomicU64 for the location of the block in the file (offset from start)
* If AtomicU64 is 0 then assume it has not yet been completed.

## Compression Workers
* Once complete they submit to writer thread, which sets the AtomicU64 to the location in the file
