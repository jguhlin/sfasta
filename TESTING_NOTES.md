# Compression Algo testing 

## UniProt Swiss-Prot Proteins
### Default Profile (zstd)
Headers Compression Ratio: 17.76% (58.38 MiB -> 10.IDs Compression Ratio: 31.52% (10.94 MiB -> 3.44 MiB) - Avg Compressed Block Size: 19.83 KiB
Sequences Compression Ratio: 29.99% (197.10 MiB -> 59.11 MiB) - Avg Compressed Block Size: 19.08 KiB

### Brotli - Default Level
[2024-07-03T23:03:20Z INFO  libsfasta::conversion] Headers Compression Ratio: 10.67% (58.38 MiB -> 6.23 MiB)
[2024-07-03T23:03:20Z INFO  libsfasta::conversion] IDs Compression Ratio: 29.44% (10.94 MiB -> 3.22 MiB) - Avg Compressed Block Size: 137.42 KiB
[2024-07-03T23:03:20Z INFO  libsfasta::conversion] Sequences Compression Ratio: 28.29% (197.10 MiB -> 55.75 MiB) - Avg Compressed Block Size: 143.82 KiB

### Gzip - Default level
[2024-07-03T23:04:52Z INFO  libsfasta::conversion] Headers Compression Ratio: 17.94% (58.38 MiB -> 10.47 MiB)
[2024-07-03T23:04:52Z INFO  libsfasta::conversion] IDs Compression Ratio: 36.50% (10.94 MiB -> 3.99 MiB) - Avg Compressed Block Size: 170.36 KiB
[2024-07-03T23:04:52Z INFO  libsfasta::conversion] Sequences Compression Ratio: 31.36% (197.10 MiB -> 61.82 MiB) - Avg Compressed Block Size: 159.45 KiB

### LZ4 - default level
[2024-07-03T23:02:35Z INFO  libsfasta::conversion] Headers Compression Ratio: 26.50% (58.38 MiB -> 15.47 MiB)
[2024-07-03T23:02:35Z INFO  libsfasta::conversion] IDs Compression Ratio: 57.71% (10.94 MiB -> 6.31 MiB) - Avg Compressed Block Size: 269.38 KiB
[2024-07-03T23:02:35Z INFO  libsfasta::conversion] Sequences Compression Ratio: 54.43% (197.10 MiB -> 107.29 MiB) - Avg Compressed Block Size: 276.74 KiB

### Snappy
[2024-07-03T23:04:24Z INFO  libsfasta::conversion] Headers Compression Ratio: 28.40% (58.38 MiB -> 16.58 MiB)
[2024-07-03T23:04:24Z INFO  libsfasta::conversion] IDs Compression Ratio: 58.94% (10.94 MiB -> 6.44 MiB) - Avg Compressed Block Size: 275.11 KiB
[2024-07-03T23:04:24Z INFO  libsfasta::conversion] Sequences Compression Ratio: 59.56% (197.10 MiB -> 117.38 MiB) - Avg Compressed Block Size: 302.78 KiB
