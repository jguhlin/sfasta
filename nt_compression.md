```
[C] 🧬                            32.00 KiB/202.07 GiB (2d)
[2023-07-28T23:18:02Z INFO  libsfasta::conversion] Writing sequences start... 109
[2023-07-29T00:37:29Z INFO  libsfasta::formats::fasta] FASTA Reading complete, returning None
[2023-07-29T00:37:29Z INFO  libsfasta::conversion] FASTA reading complete...
[2023-07-29T00:37:29Z INFO  libsfasta::conversion] fasta_thread_spins: 21916910
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] Masking time: 3856103ms
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] Adding time: 278538ms
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] SeqLoc time: 70403ms
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] Finalizing SequenceBuffer
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Waiting for compression workers to finish...
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Joining workers
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Compression worker thread spins: 47945713937
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Compression worker thread work units: 49926
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Compression worker thread spins: 47235607856
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Compression worker thread work units: 54852
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Compression worker thread spins: 48527145496
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Compression worker thread work units: 47556
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] output_spins: 14524501201
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] DEBUG: Wrote 171461509032 bytes of sequence blocks
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Compression worker thread spins: 47836808972
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Compression worker thread work units: 49799
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Sorter worker empty spins: 40832367381, have blocks spins: 177671590, output spins: 0
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] DEBUG: Writing 202133 total blocks
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] CompressionStreamBuffer finalized
[2023-07-29T00:37:38Z INFO  libsfasta::compression_stream_buffer] Emit block spins: 3868439
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] fasta_queue_spins: 2171653174
[2023-07-29T00:37:38Z INFO  libsfasta::conversion] DEBUG: Wrote 632973 bytes of block index
[2023-07-29T00:37:39Z INFO  libsfasta::conversion] DEBUG: Wrote 705452561 bytes of headers in 750.613264ms
[2023-07-29T00:37:39Z INFO  libsfasta::conversion] DEBUG: Wrote 170155379 bytes of ids in 201.732572ms
[2023-07-29T00:37:41Z INFO  libsfasta::conversion] DEBUG: Wrote 64684431 bytes of masking in 2.277249868s
[2023-07-29T00:37:42Z INFO  libsfasta::conversion] Write Fasta Sequence write time: 4779.292145551s
[2023-07-29T00:37:42Z INFO  libsfasta::conversion] Writing sequences finished... 172402434485
[2023-07-29T00:37:42Z INFO  libsfasta::conversion] Writing SeqLocs to file. 172402434485
[2023-07-29T00:47:27Z INFO  libsfasta::conversion] Writing SeqLocs to file: COMPLETE. 174603408413
[2023-07-29T00:47:27Z INFO  libsfasta::conversion] SeqLocs write time: 585.50373478s
[2023-07-29T00:47:27Z INFO  libsfasta::conversion] Writing index to file. 174603408413
[2023-07-29T00:47:28Z INFO  libsfasta::dual_level_index::dual_index] DEBUG: Hashes: 683287128 bytes
[2023-07-29T00:47:30Z INFO  libsfasta::conversion] Index write time: 3.231566206s
[2023-07-29T00:47:30Z INFO  libsfasta::conversion] Writing index to file: COMPLETE. 175632841074
[2023-07-29T00:47:30Z INFO  libsfasta::conversion] Directory write time: 11.308653ms
[2023-07-29T00:47:30Z INFO  libsfasta::conversion] DEBUG: [("Sequence Blocks", 171461509032), ("Block Index", 632973), ("Headers", 705452561), ("IDs", 170155379), ("Masking", 64684431), ("sequences", 172402434376), ("seqlocs", 2200973928), ("ind
ex", 1029432661), ("directory", 56)]
[2023-07-29T00:47:30Z INFO  libsfasta::conversion] Conversion time: 5368.05440708s
[2023-07-29T00:47:30Z INFO  sfa] Joining IO thread
[2023-07-29T00:47:30Z INFO  sfa] IO thread joined
```