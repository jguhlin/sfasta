| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `./sfa convert reads.fasta --zstd --index` | 64.555 ± 1.988 | 63.149 | 65.961 | 1.16 ± 0.04 |
| `./sfa convert reads.fasta --lz4 --index` | 55.601 ± 0.721 | 55.092 | 56.111 | 1.00 |
| `./sfa convert reads.fasta --brotli --index` | 70.478 ± 1.558 | 69.376 | 71.580 | 1.27 ± 0.03 |
| `./sfa convert reads.fasta --xz --index` | 249.099 ± 1.506 | 248.034 | 250.164 | 4.48 ± 0.06 |
