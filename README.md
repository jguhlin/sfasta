# Introduction
Sfasta is a replacement for the FASTA/Q format with a goal of both saving space but also having very fast random-access for machine learning, even for large datasets (such as the nt database, 217Gb gzip compressed, 225Gb bgzip compressed, and 175Gb with sfasta, including an index). Speed advantages are by assuming modern hardware, thus: i) multiple compression threads, ii) I/O dedicated threads, iii) SIMD bitpacking support, iv) modern compression algorithms (ZSTD, as default). If you need different hardware, open an issue. There are SIMD support for other architectures that could be implemented.

While the goals are random-access speed by ID query, and smaller size, I hope it can become a more general purpose format. Currently it makes extensive use of bitpacking, as well as ZSTD compression. It supports others, which could be used for archival purposes (such as xz compression). It is a work in progress, but is ready for community feedback. Because the goals are not simple decompression, this part of the code is not-optimized yet, and is much slower than zcat or other tools. This will be remedied in the future.

## Note
This has taken a few years of on again, off again development. FORMAT.md and other files are likely out of date.

# Usage
## Installation
`cargo install sfasta`

## Usage
To compress a file:
`sfa convert MyFile.fasta`

You can also convert directly from gzipped files
`sfa convert MyFile.fasta.gz`

You can use other compression schemes. The software automatically detects which is used and decompresses accordingly.
`sfa convert --snappy MyFile.fasta`
`sfa convert --xz MyFile.fasta`

You can also change the block size. Units are in * 1024 bytes. Default is 4Mb (4096):
`sfa convert --block-size 8192 MyFile.fasta`

View a file:
`sfa view MyFile.sfasta`

Query a file by sequence ID:
`sfa faidx MyFile.sfasta Chr1`

For help:
`sfa --help`

Please note, not all subcommands are implemented yet. The following should work: convert, view, list, faidx.
# Comparisons

## Features
| Compression Type | Random Access | Multithreaded |
|:---:|:---:|:----|
| NAF | No | No |
| ZSTD | No | Yes |
| sfasta | Yes | Yes |
| bgzip | Yes | Yes |

### Uniprot Random Access
Samtools index pre-built

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `samtools faidx uniprot_sprot.fasta.gz "sp\|Q6GZX1\|004R_FRG3G"` | 423.0 ± 3.3 | 419.4 | 434.1 | 3.42 ± 0.13 |
| `sfa faidx uniprot_sprot.sfasta "sp\|Q6GZX1\|004R_FRG3G"` | 123.8 ± 4.5 | 121.8 | 143.7 | 1.00 |

### Uniprot Compression Speed
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `sfa convert uniprot_sprot.fasta` | 1.974 ± 0.128 | 1.673 | 2.143 | 5.75 ± 0.59 |
| `ennaf --protein uniprot_sprot.fasta --temp-dir /tmp` | 0.923 ± 0.009 | 0.913 | 0.942 | 2.69 ± 0.22 |
| `bgzip -k --index -f --threads 8 uniprot_sprot.fasta` | 0.670 ± 0.019 | 0.641 | 0.734 | 1.95 ± 0.17 |
| `pigz -k -p 8 uniprot_sprot.fasta -f` | 0.636 ± 0.021 | 0.612 | 0.682 | 1.85 ± 0.16 |
| `crabz -p 8 Erow_1.0.fasta -f > uniprot_sprot.crabz` | 9.150 ± 0.182 | 8.883 | 9.523 | 26.67 ± 2.20 |
| `zstd -k uniprot_sprot.fasta -f -T8` | 0.343 ± 0.027 | 0.296 | 0.406 | 1.00 |

### Uniprot Size
Uncompressed: 282M

| Compression Type | Size |
| --- | --- |
| NAF | 68M |
| bgzip* | 92M |
| zstd | 78M | 
| sfasta | 83M |
* Excludes index

## Nanopore Reads
### Nanopore Reads Compression Speed
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `sfa convert Essy1B.fasta` | 22.844 ± 2.973 | 18.514 | 26.591 | 3.37 ± 0.45 |
| `ennaf Essy1B.fasta --temp-dir /tmp` | 19.231 ± 0.158 | 19.003 | 19.505 | 2.84 ± 0.10 |
| `bgzip -k --index -f --threads 7 Essy1B.fasta` | 82.065 ± 0.478 | 81.559 | 82.720 | 12.10 ± 0.42 |
| `pigz -k -p 7 Essy1B.fasta -f` | 83.118 ± 0.994 | 82.055 | 85.015 | 12.26 ± 0.44 |
| `crabz -p 7 Essy1B.fasta > Essy1B.fasta.crabz` | 82.933 ± 1.268 | 81.313 | 84.412 | 12.23 ± 0.46 |
| `zstd -k Essy1B.fasta -f -T7` | 6.780 ± 0.232 | 6.484 | 7.220 | 1.00 |

### Nanopore Reads Random Access
Samtools index pre-built

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `samtools faidx Essy1B.fasta.gz ae278260-d941-45c9-9e76-40f04ef8e56c` | 752.6 ± 5.8 | 746.7 | 764.8 | 9.01 ± 1.15 |
| `sfa faidx Essy1B.sfasta ae278260-d941-45c9-9e76-40f04ef8e56c` | 83.6 ± 10.6 | 71.7 | 98.5 | 1.00 |

### Nanopore Reads Size
Uncompressed Size: 8.8G
| Compression Type | Size |
| --- | --- |
| NAF | 2.2G |
| bgzip* | 2.6G |
| sfasta | 2.6G |

* Excludes index

## Genome

### Genome Random Access Speed
Samtools index pre-built

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `samtools faidx Erow_1.0.fasta.gz PXIH01S0167520.1` | 189.7 ± 10.2 | 182.2 | 215.7 | 1.53 ± 0.10 |
| `sfa faidx Erow_1.0.sfasta PXIH01S0167520.1` | 135.3 ± 9.4 | 129.4 | 153.4 | 1.09 ± 0.09 |

### Genome Compression Speed
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `sfa convert Erow_1.0.fasta` | 4.847 ± 0.189 | 4.579 | 5.199 | 2.39 ± 0.16 |
| `ennaf Erow_1.0.fasta --temp-dir /tmp` | 8.054 ± 0.044 | 7.986 | 8.155 | 3.97 ± 0.22 |
| `bgzip -k --index -f --threads 10 Erow_1.0.fasta` | 7.522 ± 0.510 | 6.639 | 8.132 | 3.71 ± 0.32 |
| `pigz -k -p 10 Erow_1.0.fasta -f` | 7.593 ± 0.110 | 7.353 | 7.965 | 3.75 ± 0.21 |
| `crabz -p 10 Erow_1.0.fasta > Erow_1.0.crabz` | 7.530 ± 0.203 | 7.317 | 7.992 | 3.72 ± 0.23 |
| `zstd -k Erow_1.0.fasta -f -T10` | 2.026 ± 0.112 | 1.834 | 2.273 | 1.00 |

### Genome Size 
Uncompressed: 2.7G

| Compression Type | Size |
|---|--|
| NAF | 446M |
| sfasta | 596M |
| bgzip* | 635M |
| Zstd | 663M |

* Excludes index

# Future Plans
## Subsequence support for faidx CLI
Only open the block(s) that contain the subsequence. The index is already set up to support this and I've had it working before in the python bindings.

## Additional Speed-ups
There is plenty of room for speed improvements, including adding more threads for specific tasks, CPU affinities, native compilation, and maybe more Cow.

## Additional Compression
There is likely room to decrease size as well without hurting speed.

## Command-line interface
As I've refactored much of the library, the CLI code withered and decayed. Need to fix this.

## Quality Scores
To support FASTQ files

## Adjust compression level and compression method
For other applications (such as long term storage)

## C and Python bindings
To make it easier to use in other programs and in python/jupyter

## Small file optimization
Sfasta is currently optimized for larger files.

## Implement NAF-like algorithm
NAF has an advantage with 4bit encoding. It's possible to implement this, and use 2bit when possible, to gain additional speed-ups. Further, there is some SIMD support for 2bit and 4bit DNA/RNA encoding.

## GFA file format support
Graph genome file format is in dire need of an optimized format

## Profile Guided Optimization
Never mind. This somehow doubled the time it takes to compress binaries. Enable PGO for additional speed-ups

# FAQ

## I get a strange symbol near the progress bar
You need to install a font that supports Unicode. I'll see if there is a way to auto-detect.

## XZ compression is fast until about halfway, then slows to a crawl.
The buffers can store lots of sequence, but the compression algorithm takes longer.

## Why so many dependencies?
Right it it works with a wide range of compression functions. Once some are determined to be the best others could be dropped from future versions. The file format itself has a version identifier so we could request people rollback to an older version if they need to.

![Genomics Aotearoa](info/genomics-aotearoa.png)
