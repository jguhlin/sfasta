
# SFASTA File Format Description

| Category | Item | type | Description |
| -------- | ---- | ---- | ----------- |
| Header   |      |      |             |
|          | "sfasta" | \[u8\] | The string "sfasta" as bytes to indicate an sfasta file |
|          | Sfasta Version | u64 | Sfasta Version as u64, for future compatability |
| | Directory | Directory | Directory struct |
| | Parameters | Parameters | Parameters Struct |
| | Metadata | Metadata | Metadata Struct |
| Sequence Blocks | | SequenceBlockCompressed | Individual SequenceBlockCompressed |
| Score Blocks | SequenceBlockCompressed | | TODO |
| SeqInfo Blocks | SequenceBlockCompressed| | TODO |
| SeqLoc Blocks | | Vec\<Vec\<Loc\>\> | Location of each of the sequences, stored as groups of Vec\<Loc\> |
| Index | | StoredIndexPlan | How to reassemble the index |
| Index | Parts | &\[u64\] | Index Hashes but split into parts |
| Index | Bitpacked | Vec\<Bitpacked\> | Bitpacked part of the index |
| Index | ID Blocks | | |
| Index | Id Block Locs | | |
| Index | Bitpacked ID Block Locs | | |
| SeqLoc| | SeqLoc Block Locs | |
| Block Index | | |

## Sequence Blocks
*customizable compression, zstd default*

Contiguous blocks of any type of sequence (nucleic, protein, etc... but also sequence header data, masking, scores, etc...)

Primarily created by the `SequenceBlockCompressed` type. 

Sequences are stored as compressed blocks, in order. For each entry, the ID is returned along with a Vec of Locations. So if sequence "MySequence" is stored in blocks 4 from position 500-1000, and 5, with position 1-250, then the ID and Locations are:

```
String<"MySequence">, Vec<(4, (500, 1000)), (5, (1, 250))>
```

Allowing for fast decompression access

## Sequence Location (SeqLoc) Blocks
*zstd level -3*

SeqLocs (Locations, ex: Vec<(4, (500, 1000)), (5, (1, 250))>) are stored as compressed blocks in lengths of SEQLOCS_CHUNK_SIZE. It takes ```Vec<(String, Location)>``` and turns it into ```Vec<String>```. The location of String matches the location of the SeqLoc, and the block to decompress is the ordinal of the specific String / SEQLOCS_CHUNK_SIZE, as an integer, and the specific location is the will be the remainder of that operation (```String Ordinal % SEQLOCS_CHUNK_SIZE```). 

The location of each SeqLoc Block is stored in SeqLoc Blocks Locs vector (see below).

SeqLocs are separated from the index to allow for different types of access patterns (full-text search, for example) while allowing for exact-match to remain speedy.

## Multi-leveled Index
*Exact match (uppercase) index*

Because the NT database and sequencing files are often very large, the index is multi-level so only the appropriate parts are queried. *This is the part that took the longest!*

Index64 (default, only supported one at this time) is each sequence ID is stored as a hash (default: xxhash64), and locations stored as a u32, which points to the original ordinal location.

### Storage
*hashes: zstd compression level -9*
Index is stored as blocks of sorted hashes, with each starting hash stored as the upper layer. Locations (ordinal location of the SeqLoc block, specifically) are stored as u32, bitpacked.

Upper layer is stored as a ```Vec<u64>```, which is then bitpacked.

### ID Blocks
*zstd compression level 11*
The original IDs are stored separately, in compressed blocks of IDX_CHUNK_SIZE, in ordinal position. These are stored both to recreate the original ID when viewing as a traditional FASTA/Q file, 

# How the Index Works 2022
The upper layer index is decompressed as a ```Vec<u64>``` when the file is initially opened.

ID is converted to upper case, then hashed with xxhash64. The hash is then used to find the ordinal location in the upper layer index. The hash is located in the block between two hashes of the upper index. The sorted hash block is calculated using IDX_CHUNK_SIZE, and then each hash in the block is compared to the searched hash. If the hash is found, the original ordinal location is found in the bitpacked block.

The appropriate ID block is decompressed, and the ID is compared to the entered ID (case-sensitive) to prevent any hash collisions. If matched, the SeqLoc location is then decompressed from the ordinal and that is used to identify the locations of sequence, scores, masking, etc...

## Dual Index
* locs_start
* block_locs
* bitpacked data (many)
* blocks_locs_loc
* end

# Previous attempt: How the Index works
IDs are hashed and stored as Vec\<u64\>, locs are stored as Vec\<u32\> and point to the location of the seqloc data. Index.add(id, loc) -> loc is the # of the SeqLoc. SeqLocs are then divided in SEQLOCS_CHUNK_SIZE and stored.

IDs are stored as Vec\<u64\> and binary search is performed to quickly find the position, opposed to a HashMap which is more memory intensive and slower. This also means hashes are **not restricted to be unique**. So the find function can return multiple locations.

Seq Locs are Vec\<Loc\> and stored in blocks of SEQLOCS_CHUNK_SIZE, so that we do not have to deserialize the entire vector into memory to access only a single location. 

## Summary
ID -> Hash -> SeqLoc Location -> Identify SeqLoc Block -> Deserialize Block -> Get proper SeqLoc -> SeqLoc specifies Sequence Block and boundaries of sequence.

## Weaknesses
This does not support string matching for sequence IDs. If needed, this should be implemented another way.


Index hashes are split into smaller parts (up to 255, u8::MAX)

Parts have locations, can identify which block contains the hash, and then just decompress that, to identify where the index part is.





# Structs

Loc...
Block, Start, End...

Directory...

Parameters...

Metadata....

SequenceBlockCompressed

StoredIndexPlan
