# 0.3.4 
## Major Changes
Breaks file format, please convert to FASTA then back to SFA
Far better CPU handling
Updated FASTQ file parser
Prefetching data for speeding up viewing entire files sequentially
Slice SeqLoc's to get sub-sequences (no more decompressing entire genomes for a small fraction)
Fuzzing for parse_fasta and parse_fastq
More efficient store of SeqLoc's for large files


## Minor Changes
Print out the correct version from cargo.toml
Less allocations for FASTA file parser
Util fn's to detect file type and some compression types
Switch project back to using rust workspace
Some refactoring for WASM support - WIP
Added convenience functions

# 0.3.3
Remove requirement for nightly rust compiler.