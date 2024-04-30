# Introduction
Sfasta is a replacement for the FASTA/Q format with the twin goals of saving space and having very fast random-access, even for large datasets (such as the nt database, 203Gb gzip compressed, 210Gb bgzip compressed(+1.8Gb index), and 141Gb with sfasta, index inclusive). 

Speed comes from assuming modern hardware, thus: 
* Multiple compression threads by default
* Dedicated I/O Threads
* Modern compression algorithms (ZSTD, as default)
* Fractal Index
* Everything stored as sequence streams (like stored with like, for better compression)

The goals are random-access speed by query or random, and smaller size. It supports other compression algorithms, which could be used for archival purposes (such as xz compression).

## Community Feedback Period
I'm hopeful folks will check this out, play around, break it, and give feedback. 

# Usage
## Installation
`cargo install sfasta` [Don't have cargo?](https://rustup.rs/)

## Usage

### Compression
To compress a file:
```bash
sfa convert MyFile.fasta

#You can also convert directly from gzipped files:
sfa convert MyFile.fasta.gz
```

Compression profiles are supported. The built-in ones can be accessed with --fast, --fastest, --small, --smallest.
```bash
sfa convert --fast reads.fastq
sfa convert --fastest reads.fastq
```

Fast / Fastest are optimized for fast reading and random access, while Small / Smallest are optimized for size of file on disk. *These are in development, let me know what works best for you*

You can specify your own profile, using a template from [GitHub](https://github.com/jguhlin/sfasta) as an starting point:
```
sfa convert -p myprofile.yml reads.fastq
```

You can use other compression schemes. The software automatically detects which is used and decompresses accordingly.

```bash
sfa convert --snappy MyFile.fasta
sfa convert --xz MyFile.fasta

# Reading the file "just works"
sfa view MyFile.sfasta 
```

You can also change the block size. Smaller blocks will speed up random access, while larger blocks will increase compression ratios. 512 (512kb, 524288 bytes) is default.

```bash
# This will create 8Mb blocks
sfa convert --block-size 8192 MyFile.fasta
```

View a file:
```bash
sfa view MyFile.sfasta
```

Query a file by sequence ID:
```bash
sfa faidx MyFile.sfasta Chr1
```

For help:
```bash
sfa --help
```

## Requirements
Should work anywhere that supports [Rust](https://www.rust-lang.org/). Tested on Ubuntu, Red Hat, and Windows 10. I suspect it will work on Mac OS. WASM support is forthcoming.

# Details

## Compression Types Supported
| Type | Status | Notes |
|:-----|:------:|:-----:|
| ZSTD | Default | Optimized |
| Brotli | Implemented | Could be more optimized |
| LZ4 | Implemented | Rust implementation does not support levels |
| XZ | Implemented | Slow, high compression |
| Snappy | Implemented | Not recommended |
| GZIP | Implemented | Not recommended |
| BZIP2 | Implemented | Not recommended |
| NONE | | No compression, supported |

*Compression can be set per data type*
- You can use ZSTD for sequence compression, XZ for masking, Brotli for IDs and Headers, for example.
- Compression profiles are found in [compression_profiles](https://github.com/jguhlin/sfasta) folder and are stored as YAML.
- You can load your own compression profile from the command line.
- IDs are interpreted from FASTA/Q files as the part before the space on an identified line, for example
```
>Chr1 This is the chromosome
^---^ ^--------------------^
 ID    Header
```
The same rule applies for FASTQ.

## Data Types Supported

| Data | Status |
|:----:|:------:|
| Sequences | Fully supported, nucleotide, amino, and RNA (anything, really) |
| Sequence IDs | Fully Supported |
| Additional Header Information | Fully Supported |
| Quality Scores | Fully Supported |
| Masking | Fully Supported |
| Flags | Planned |
| Nanopore Signals | Planned |
| Base Modifications | Planned |
| Pangenome Graph | Maybe |
| Alignments | Not Planned. CRAM fulfills this role. |
| Variants | Too different, another solution needed. |


# Comparisons

## Features
| Compression Type | Random Access | Multithreaded | Tools |
|:---:|:---:|:----:|:-----:|
| sfasta | Yes | Yes | sfa |
| gzip | No | Yes | gzip, pigz, crabz |
| [bgzip](http://www.htslib.org/doc/bgzip.html) | Yes | Yes | bgzip, crabz |
| [NAF](https://github.com/KirillKryukov/naf) | No | No | naf |
| [ZSTD](http://facebook.github.io/zstd/) | No | Yes | zstd |

## Benchmarks

### Uniprot Random Access
Using [UniProt SWISS-Prot release 2024_02](https://www.uniprot.org/help/downloads). File size is ~88Mb. 

Samtools index pre-built with ```samtools faidx uniprot_sprot.fasta.gz``` with a bgzip compressed file.

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `sfa faidx uniprot_sprot.sfasta "sp\|Q8CIX8\|LGSN_MOUSE" "sp\|O31861\|YOJB_BACSU" "sp\|B2XTX0\|PSAJ_HETA4" "sp\|A2YNP0\|SPX6_ORYSI" "sp\|P69474\|CAPSD_CGMVS" ` | 16.5 ± 0.3 | 15.7 | 18.2 | 1.00 |
| `samtools faidx uniprot_sprot.fasta.gz "sp\|Q8CIX8\|LGSN_MOUSE" "sp\|O31861\|YOJB_BACSU" "sp\|B2XTX0\|PSAJ_HETA4" "sp\|A2YNP0\|SPX6_ORYSI" "sp\|P69474\|CAPSD_CGMVS" ` | 456.9 ± 6.8 | 451.8 | 474.9 | 27.75 ± 0.66 |

### Uniprot Size
Uncompressed: 272M

| Compression Type | Size |
| --- | --- |
| sfasta (index incl) | 71Mb |
| NAF (no index) | 66Mb |
| zstd (no index) | 76Mb | 
| bgzip (excl. index) | 88Mb |
| bgzip (incl. index) | 111Mb (88Mb + 23Mb)
| pigz (no index) | 89Mb |

* SFASTA contains an index, where the other formats do not. An fai index is 23Mb for this uniprot.
* SFASTA uses a higher compression level for zstd as default, as well as stream compression like NAF, thus the smaller size.

```
❯ ls -lah uniprot_sprot.fasta.gz.fai
-rw-rw-r-- 1 joseph joseph 23M Apr 26 17:02 uniprot_sprot.fasta.gz.fai
```

### UniProt Size (sfa profiles)
| Profile | Size |
|:-------:|:----:|
| smallest | 59Mb |
| small | 62Mb |
| default | 68Mb |
| fast | 73Mb |
| fastest | 78Mb |

### Uniprot Compression Speed
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `sfa convert --threads 14 uniprot_sprot.fasta` | 1.543 ± 0.057 | 1.458 | 1.648 | 3.67 ± 0.34 |
| `bgzip -kf --threads 16 uniprot_sprot.fasta` | 1.293 ± 0.042 | 1.187 | 1.345 | 3.08 ± 0.28 |
| `pigz -kf -p 16 uniprot_sprot.fasta` | 0.959 ± 0.017 | 0.934 | 0.990 | 2.28 ± 0.20 |
| `ennaf --protein --temp-dir /tmp uniprot_sprot.fasta -o uniprot_sprot.naf` | 1.078 ± 0.008 | 1.069 | 1.094 | 2.57 ± 0.22 |
| `zstd -k uniprot_sprot.fasta -f -T16` | 0.420 ± 0.035 | 0.363 | 0.485 | 1.00 |
| `crabz -f bgzf -p 16 uniprot_sprot.fasta -o uniprot_sprot.fasta.gz` | 0.674 ± 0.017 | 0.649 | 0.703 | 1.60 ± 0.14 |

Compression speed is slower, but this is primarily due to the index creation. For bgzip samtools takes 2.07 seconds to generate the index. Also of note is pigz, ennaf, and zstd do not support indexing, while crabz does (using bgzf format).

## Nanopore Reads
As a FASTA file.

### Nanopore Reads Compression Speed
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `sfa convert nanopore.fasta` | 22.844 ± 2.973 | 18.514 | 26.591 | 3.37 ± 0.45 |
| `ennaf nanopore.fasta --temp-dir /tmp` | 19.231 ± 0.158 | 19.003 | 19.505 | 2.84 ± 0.10 |
| `bgzip -k --index -f --threads 7 nanopore.fasta` | 82.065 ± 0.478 | 81.559 | 82.720 | 12.10 ± 0.42 |
| `pigz -k -p 7 nanopore.fasta -f` | 83.118 ± 0.994 | 82.055 | 85.015 | 12.26 ± 0.44 |
| `crabz -p 7 nanopore.fasta > nanopore.fasta.crabz` | 82.933 ± 1.268 | 81.313 | 84.412 | 12.23 ± 0.46 |
| `zstd -k nanopore.fasta -f -T7` | 6.780 ± 0.232 | 6.484 | 7.220 | 1.00 |

### Nanopore Reads Random Access
Samtools index pre-built

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `sfa faidx nanopore.sfasta ae278260-d941-45c9-9e76-40f04ef8e56c` | 83.6 ± 10.6 | 71.7 | 98.5 | 1.00 |
| `samtools faidx nanopore.fasta.gz ae278260-d941-45c9-9e76-40f04ef8e56c` | 752.6 ± 5.8 | 746.7 | 764.8 | 9.01 ± 1.15 |

### Nanopore Reads Size
Uncompressed Size: 8.8G
| Compression Type | Size |
| --- | --- |
| NAF (no index) | 2.2G |
| sfasta (incl index) | 2.6G |
| bgzip (excl index) | 2.6G |
| zstd (no index) | 2.7G |

## Genome

### Genome Random Access Speed
Samtools index pre-built

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `sfa faidx Erow_1.0.sfasta PXIH01S0167520.1` | 135.3 ± 9.4 | 129.4 | 153.4 | 1.09 ± 0.09 |
| `samtools faidx Erow_1.0.fasta.gz PXIH01S0167520.1` | 189.7 ± 10.2 | 182.2 | 215.7 | 1.53 ± 0.10 |

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
| NAF (no index) | 446M |
| sfasta (incl index) | 596M |
| bgzip (excl index) | 635M |
| Zstd (no index) | 663M |

## Illumina Reads
11Gb of reads (FASTQ)

### Compression Speed
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `sfa convert --threads 14 reads.fastq` | 80.794 ± 3.379 | 77.700 | 87.570 | 9.59 ± 0.53 |
| `bgzip -kf --threads 16 reads.fastq` | 48.920 ± 0.264 | 48.645 | 49.279 | 5.81 ± 0.21 |
| `pigz -kf -p 16 reads.fastq` | 80.712 ± 1.461 | 78.945 | 83.181 | 9.58 ± 0.39 |
| `ennaf --dna --fastq --temp-dir /tmp reads.fastq -o reads.naf` | 38.276 ± 0.246 | 37.951 |
38.602 | 4.54 ± 0.17 |
| `zstd -k reads.fastq -f -T16` | 8.423 ± 0.303 | 7.915 | 8.889 | 1.00 |
| `crabz -f bgzf -p 16 reads.fastq -o reads.fastq.gz` | 22.176 ± 3.802 | 17.889 | 26.813 | 2.63 ± 0.46 |

### Genome Size 
Uncompressed: 2.7G

| Compression Type | Size |
|---|--|
| NAF (no index) | 1.9Gb |
| sfasta (incl index) | 2.5Gb |
| bgzip (excl index) | 635M |
| Zstd (no index) | 663M |


# Future Plans
## Implement NAF-like algorithm
[NAF](https://github.com/KirillKryukov/naf) has an advantage with 4bit encoding. It's possible to implement this, and use 2bit when possible, to gain additional speed-ups. Further, there is some SIMD support for 2bit and 4bit DNA/RNA encoding.

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

## GFA file format support
Graph genome file format is in dire need of an optimized format

## Profile Guided Optimization
Never mind. This somehow doubled the time it takes to compress binaries. Enable PGO for additional speed-ups

# Fuzzing

The following format parsers are fuzzed. To fuzz execute the following in the libsfasta directory:
```
cargo fuzz run parse_fastq  -- -max_len=8388608
cargo fuzz run parse_fasta  -- -max_len=8388608
cargo fuzz run parse_sfasta -- -detect_leaks=0 -rss_limit_mb=4096mb -max_len=8388608
```

# FAQ

## I get a strange symbol near the progress bar
You need to install a font that supports Unicode. I'll see if there is a way to auto-detect.

## Why so many dependencies?
Right it it works with a wide range of compression functions. Once some are determined to be the best others could be dropped from future versions. The file format itself has a version identifier so we could request people rollback to an older version if they need to.

## Why samtools comparison?
I've got plenty of experiments trying to get a fast gzip compressed multi-threaded reader, but even when mounted on a ramdisk, it is too slow. Samtools is an awesome, handy tool that has the 'faidx' function, which I use almost constantly. While faidx is a handy utility function, it is not optimized for large datasets, thus the test is a little unfair. Still, it's helpful to have something to compare to. 

# File Format
The format is found in the docs.

![Genomics Aotearoa](./static/genomics-aotearoa.png)
