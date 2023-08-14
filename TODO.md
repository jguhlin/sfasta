* Adding to SeqLocs is slow. SeqLocs write_to_buffer is slow. Benchmark and try.
* Pin compression workers to CPUs
* Use syncmers to group together similar sequences ( Only useful for very large blocks though )
* Use Pulp to speed up on certain platforms
* Magic chunkify struct
* All prefetch functions should coerce a BufReader
* Is stream vbytes better than zstd for masking?