# Introduction
Sfasta is a replacement for the FASTA/Q format with the twin goals of saving space and having very fast random-access, even for large datasets (such as the nt database, 203Gb gzip compressed, 210Gb bgzip compressed(+1.8Gb index), and 141Gb with sfasta, index inclusive). 

Speed comes from assuming modern hardware, thus: 
* Multiple threads by default
* Dedicated I/O Threads
* Modern compression algorithms (ZSTD, as default)
* Fractal Index
* Everything stored as sequence streams (for better compression)

While the goals are random-access speed by ID query, and smaller size, I hope it can become a more general purpose format. It supports other compression algorithms, which could be used for archival purposes (such as xz compression). This is a work in progress, but ready for community feedback. 

## Community Feedback Period
I'm hopeful folks will check this out, play around, break it, and give feedback. 

## Data Types Supported

| Data | Status |
|:----:|:------:|
| Sequences | Fully supported |
| Sequence IDs | Fully Supported |
| Additional Header Information | Fully Supported |
| Quality Scores | Fully Supported |
| Masking | Fully Supported |
| Nanopore Signals | Planned |
| Base Modifications | Planned |

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

You can use other compression schemes. The software automatically detects which is used and decompresses accordingly.

```bash
sfa convert --snappy MyFile.fasta
sfa convert --xz MyFile.fasta

# Reading the file "just works"
sfa view MyFile.sfasta 
```

You can also change the block size. Smaller blocks will speed up random access, while larger blocks will increase compression ratios. 512 (512kb, 524288 bytes) is default.

```bash
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

Please note, not all subcommands are implemented yet. The following should work: convert, view, list, faidx.

## Requirements
Should work anywhere that supports [Rust](https://www.rust-lang.org/). Tested on Ubuntu, Red Hat, and Windows 10. I suspect it will work on Mac OS. WASM support is forthcoming.

# Comparisons

## Features
| Compression Type | Random Access | Multithreaded | Tools |
|:---:|:---:|:----:|:-----:|
| sfasta | Yes | Yes | sfa |
| gzip | No | Yes | gzip, pigz, crabz |
| [bgzip](http://www.htslib.org/doc/bgzip.html) | Yes | Yes | bgzip, crabz |
| [NAF](https://github.com/KirillKryukov/naf) | No | No | naf |
| [ZSTD](http://facebook.github.io/zstd/) | No | Yes | zstd |

# Future Plans
## Speed
- Final Index generation and compression is the most time consuming task. It can be parallelized.

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

## XZ compression is fast until about halfway, then slows to a crawl.
The buffers can store lots of sequence, but the compression algorithm takes longer.

## Why so many dependencies?
Right it it works with a wide range of compression functions. Once some are determined to be the best others could be dropped from future versions. The file format itself has a version identifier so we could request people rollback to an older version if they need to.

## Why samtools comparison?
I've got plenty of experiments trying to get a fast gzip compressed multi-threaded reader, but even when mounted on a ramdisk, it is too slow. Samtools is an awesome, handy tool that has the 'faidx' function, of which I am a huge fan. While the faidx is not it's main function, it is not optimized for large datasets, thus the test is a little unfair. Still, it's helpful to have something to compare to. 

# File Format
The best source is currently this file: [conversion.rs](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/conversion.rs#L148). Masking is converted into instructions in a u32, see [ml32bit.rs](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/masking/ml32bit.rs).

## Header
| Data Type | Name | Description |
| ---:|:--- |:--- |
| [u8; 6] | b"sfasta" | Indicates an sfasta file |
| u64 | Version | Version of the SFASTA file (currently set to 1) |
| struct | [Directory](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/directory.rs#L53) | Directory of the file; u64 bytes pointing to indices, sequence blocks, etc... |
| struct | [Parameters](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/parameters.rs#L5) | Parameters used to create the file |
| struct | [Metadata](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/metadata.rs#L3) | unused |
| structs | [SequenceBlock](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/sequence_block.rs#L11) | Sequences, split into block_size chunks, and compressed on disk (see: [SequenceBlockCompressed](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/sequence_block.rs#L154)) |
| Vec<u64> | Sequence Block Offsets | Location of each sequence block, in bytes, from the start of the file. Stored on disk as bitpacked u32. |
| [Header](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/header.rs) Region ||
| enum | [CompressionType](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/structs.rs#L23) | Type of compression used for the Headers. |
| u64 | Block Locations Position | Position of the header blocks locations |
| [u8] | Header Block | Header (everything after the sequence ID in a FASTA file) stored as blocks of u8 on disk, zstd compressed. |
| [u64] | Header Block Offsets | Location of each header block, in bytes, from the start of the file. |
| [IDs](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/id.rs) Region ||
| enum | [CompressionType](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/structs.rs#L23) | Type of compression used for the IDs. |
| u64 | Block Locations Position | Position of the ID blocks locations |
| [u8] | ID Block | IDs (everything after the sequence ID in a FASTA file) stored as blocks of u8 on disk, zstd compressed. |
| [u64] | ID Block Offsets | Location of each header block, in bytes, from the start of the file. |
| [Masking](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/masking.rs) Region ||
| u64 | bitpack_len | Length of each bitpacked block |
| u8 | num_bits| Number of bits used to bitpack each integer |
| [Packed] | BitPacked Masking Instructions | Bitpacked masking instructions. [See here](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/masking/ml32bit.rs) |
| [struct] | [SeqLocs](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/data_types/sequence_block.rs) | Sequence locations, stored as a vector of u64. |
| Special | [Dual Index](https://github.com/jguhlin/sfasta/blob/main/libsfasta/src/dual_level_index/dual_index.rs) | See file for more description. Rest of this table TBD |

![Genomics Aotearoa](./static/genomics-aotearoa.png)
