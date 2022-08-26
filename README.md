# Introduction
Sfasta is a side-project I've been working on. I'm training Transformer models and needed to use large datasets such as reads or databases. Block gzip, even when stored on a ramdisk, is far too slow to keep the GPU full, so I started working on an alternative format that allowed for better random-access. This is that format.

I hope it becomes a more general purpose format, saving space and time, but for me it has been a success for my needs already.

Currently it makes extensive use of bitpacking, as well as ZSTD compression. It supports others, which could be used for archival purposes (such as xz compression), but those functions are not connected to the command-line anymore. Size is generally favorable as-is, although disabling masking and indexing would make it even smaller.

# Format Overview
Everything is bincoded, and either bitpacked, or ZSTD compressed.... add more here...

# Comparisons

## Features
| Compression Type | Random Access | Multithreaded |
|:---:|:---:|:----|
| NAF | No | No |
| ZSTD | No | Yes |
| sfasta | Yes | Yes |
| bgzip | Yes | Yes |


## Genome

### Genome Compression Speed
| Command | Mean [s] | Min [s] | Max [s] | Relative |
|:---|---:|---:|---:|---:|
| `ennaf Erow_1.0.fasta --temp-dir /tmp` | 8.111 ± 0.018 | 8.087 | 8.145 | 3.65 ± 0.11 |
| `bgzip -k --index -f --threads 7 Erow_1.0.fasta` | 9.492 ± 0.442 | 8.940 | 9.975 | 4.27 ± 0.24 |
| `sfa convert Erow_1.0.fasta` | 7.499 ± 0.477 | 6.914 | 8.345 | 3.37 ± 0.24 |
| `pigz -k -p 7 Erow_1.0.fasta -f` | 10.833 ± 0.271 | 10.548 | 11.213 | 4.87 ± 0.19 |
| `crabz -p 7 Erow_1.0.fasta > Erow_1.0.crabz` | 10.594 ± 0.171 | 10.387 | 10.971 | 4.76 ± 0.16 |
| `zstd -k Erow_1.0.fasta -f -T7` | 2.224 ± 0.067 | 2.087 | 2.291 | 1.00 |

### Genome Size 
Uncompressed: 2.7G

| Compression Type | Size |
|---|--|
| NAF | 446M |
| sfasta | 596M |
| bgzip | 635M |
| Zstd | 663M |

### Uniprot Random Access
| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `samtools faidx uniprot_sprot.fasta.gz "sp\|P10459\|3S1B_LATLA"` | 422.7 ± 4.7 | 417.8 | 433.3 | 3.42 ± 0.25 |
| `sfa faidx uniprot_sprot.fasta.sfasta "sp\|P10459\|3S1B_LATLA"` | 123.5 ± 8.9 | 118.3 | 141.6 | 1.00 |

### Uniprot Compression Speed
| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `ennaf --protein uniprot_sprot.fasta --temp-dir /tmp` | 966.5 ± 32.2 | 924.6 | 1018.7 | 2.89 ± 0.18 |
| `bgzip -k --index -f --threads 7 uniprot_sprot.fasta` | 719.7 ± 7.6 | 706.6 | 731.0 | 2.16 ± 0.12 |
| `sfa convert uniprot_sprot.fasta` | 2676.7 ± 105.9 | 2394.8 | 2756.2 | 8.02 ± 0.53 |
| `pigz -k -p 7 uniprot_sprot.fasta -f` | 771.5 ± 55.2 | 704.2 | 872.3 | 2.31 ± 0.21 |
| `crabz -p 7 Erow_1.0.fasta -f > uniprot_sprot.crabz` | 10688.8 ± 224.7 | 10322.6 | 10990.7 | 32.01 ± 1.82 |
| `zstd -k uniprot_sprot.fasta -f -T7` | 333.9 ± 17.6 | 303.4 | 351.6 | 1.00 |

### Uniprot Size
Uncompressed: 282M

| Compression Type | Size |
| --- | --- |
| NAF | 68M |
| bgzip | 92M |
| zstd | 78M | 
| sfasta | 83M |

## Nanopore Reads
### Nanopore Reads Compression Speed

### Nanopore Reads Random Access

### Nanopore Reads Size
Uncompressed Size: 8.8G

# Future Plans
## Quality Scores
To support FASTQ files

## Adjust compression level and compression method
For other applications (such as long term storage)

## C and Python bindings
To make it easier to use in other programs and in python/jupyter

## PGO
Enable PGO for additional speed-ups