# SFASTA File Format Definition
Version 0.0.3

## Rasoning / Need
FASTA/Q format is slow for random access, even with an index provided by samtools.

## Pros/Cons
By concatenating all of the sequences into successive blocks, it becomes more difficult to add or remove sequences at a whim. However, many large sequence files are rarely changed (NT, UniProt, nr, reads, etc).

### Overview
* Directory struct
* Parameters struct
* Metadata struct
* Sequence streams
* Scores stream
* Masking Stream
* Index struct
* Order Struct

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


### Directory
```
Directory { 
    index_loc: u64,
    seq_loc: u64,
    scores_loc: Option(u64),
    // seqinfo_loc: Option(u64),  // Not yet...
}
```

Address of various contents of the file. 

Index loc: Location of the index.  
Seq Loc: Location of the sequence stream.  
Scores Loc: (Optional) Location of the scores stream.  

### Parameters
```
Parameters { 
    block_size: u32,
    compression_type: enum { Zstd, NAF, Lz4, Bzip2, Gzip, ... },
}
```

Block size is how much sequence is compressed per block, represented by bytes. Default is 4Mbp..... option to benchmarking.

Compression type is an ENUM of the compression algorithm used. Zstd is recommended.

### Metadata
```
Metadata {
    created_by: String,
    citation_doi: Option(String),
    citation_url: Option(String),
    citation_authors: Option(String),
    date_created: u64,
    title: Option(String),
    description: Option(String),
    notes: Option(String),
    download_url: Option(String),
    homepage_url: Option(String),
}
```

### Sequence Stream
Biological sequences (reads, amino acids, nucleotides, DNA, RNA) are appended to a string of maximum size of block_size, and compressed and stored as a data structur

```
SequenceBlock {
    compressed_seq: \[u8\],
}
```

Each sequence can be addressed by the relevant block_id's and the byte-offset. For example a sequence stored in blocks 3, 4, and 5 starting in block 3 at position 1,024,048 and ending in block 5 at position 1,042 would encompass all of block 4.

### Scores Stream
```
ScoresBlock {
    compressed_seq: \[u8\],
}
```

Optional. See @Sequence Stream

### Seqinfo Stream
TBD....

Per sequence metadata? That'd be bad for compression though...
Open string all concat'd? Then it's unstructued...

### Index
Goal is to **not** store all of NT (for example) in a hashmap in memory at once.

#### Alternate Index Type

# TODO:
Multiple types of index. "Fast" is uncompressed. "Packed" is compressed with Zstd, maybe even not hashmapped.
Also supports hashed at 64-bit and 32-bit, and 16-bit for very small files.

#### Index Directory
```
IndexDirectory {
    id_index: Option(u64),
    block_index: Option(u64),
    scores_block_index: Option(u64),
    masking_block_index: Option(u64),
}
```

Where u64 is the location from the index directory location.

#### ID Index

Index is stored as compressed blocks of 256(?) elements.
```
IndexBlock {
    first: u64,
    end: u64,
    compressed_index: Vec<u8>
}
```

First and end are the XxHash encoded in the case of IDs.

Where compressed index is a bincode-encoded zstd compressed:

```
Index => HashMap<id, location> where
  id: String
  location: (Vec<(u32, (u64, u64))>, Option<Vec<(u32, (u64, u64))>>)
```

Second option and locs above are for scores (None if no scores)

#### NOT USING BELOW

#### Need multiple indices, one for blocks, one for score blocks, one for seq ID's...

```
IndexMetadata {
    index_levels: u8,
}
```

Index levels are how many levels the index branches go before becoming "leafs" which serve as endpoints.

```
Index {
    idx: HashMap(\[u8\], struct(Index OR IndexLeaf)),
}
```

```
IndexLeaf {
    idx: HashMap(\[u8\], Info),
}
```

```
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
```

### Future
Make it easier to add / remove sequences...
Add a separate area of sequence headers? Right now to list all sequences in a file must read the entire index and print out the info. On the other hand, as long as they are properly chopped up, it should be small...

# Changes
## Version 0.0.2
Merge index into file. Split sequences, scores, and seq info into separate streams for more efficient compression.

## Version 0.0.1
Only optimized for larger sequences and was very slow for reads. File size was larger than gzip despite having better compression.