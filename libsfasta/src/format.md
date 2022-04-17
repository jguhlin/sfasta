
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



# How the Index works
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
