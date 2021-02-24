# SFASTA File Format Definition
Version 0.0.2

## Reason
FASTA/Q format is slow for random access, even with an index provided by samtools.

## Pros/Cons
By concatenating all of the sequences into successive blocks, it becomes more difficult to add or remove sequences at a whim. However, many large sequence files are rarely changed (NT, UniProt, nr, reads, etc).

## Warning
This is a format still in heavy development, backwards compatability is specifically not guaranteed, nor desirable at this stage.

## File Format
Bincoded, using serde.

### Location of index
Directory { 
    index_loc: u64,
    seq_loc: u64,
    scores_loc: Option(u64),
    seqinfo_loc: Option(u64),
}

Address of various contents of the file. Written as u64::MAX-1 at first, then overwritten once file creation is completed.

Index loc: Location of the index.
Seq Loc: Location of the sequence stream.
Scores Loc: (Optional) Location of the scores stream.
SeqInfo Loc: (Optional) Location of the sequence info stream.

### Parameters
Parameters { 
    block_size: u32,
    compression_type: enum { Zstd, NAF, Lz4, Bzip2, Gzip, ... },
}

Block size is how much sequence is compressed per block, represented by bytes. Default is 4Mbp..... option to benchmarking.

Compression type is an ENUM of the compression algorithm used. Zstd is recommended.

### Metadata
Metadata {
    created_by: String,
    citation_doi: Option(String),
    citation_url: Option(String),
    citation_authors: Option(String),
    date_created: u64,
    title: Option(String),
    description: Option(String),
    notes: Option(String),
}

### Sequence Stream
Biological sequences (reads, amino acids, nucleotides, DNA, RNA) are appended to a string of maximum size of block_size, and compressed and stores as a data structure.

SequenceBlock {
    block_id: u64,
    compressed_seq: \[u8\],
}

Each sequence can be addressed by the relevant block_id's and the byte-offset. For example a sequence stored in blocks 3, 4, and 5 starting in block 3 at position 1,024,048 and ending in block 5 at position 1,042 would encompass all of block 4.

### Scores Stream
ScoresBlock {
    block_id: u64,
    compressed_seq: \[u8\],
}

Optional. See @Sequence Stream

### Seqinfo Stream
TBD....

Per sequence metadata? That'd be bad for compression though...
Open string all concat'd? Then it's unstructued...

### Index
Goal is to not store all of NT (for example) in a hashmap in memory at once.

IndexMetadata {
    index_levels: u8,
}

Index levels are how many levels the index branches go before becoming "leafs" which serve as endpoints.

Index {
    idx: HashMap(\[u8\], struct(Index OR IndexLeaf)),
}

IndexLeaf {
    idx: HashMap(\[u8\], Info),
}

Info {
    id: String, // Sequence ID
    seq_start: (block_id, u32),
    seq_end: (block_id, u32),
    scores_start: Option(block_id, u32),
    scores_end: Option(block_id, u32),
    seqinfo_start: TBD,
    seqinfo_end: TBD,
    seqtype: ENUM( DNA, RNA, Protein ),
    seqlength: u64,
}

### Future
Make it easier to add / remove sequences...
Add a separate area of sequence headers? Right now to list all sequences in a file must read the entire index and print out the info. On the other hand, as long as they are properly chopped up, it should be small...

# Changes
## Version 0.0.2
Merge index into file. Split sequences, scores, and seq info into separate streams for more efficient compression.

## Version 0.0.1
Only optimized for larger sequences and was very slow for reads. File size was larger than gzip despite having better compression.