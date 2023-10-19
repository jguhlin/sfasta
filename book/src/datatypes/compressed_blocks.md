# Compressed Blocks

Compressed blocks are contiguous blocks of data compressed with the [specified compression algorithm and level](./parameters.md). These blocks are stored as they are created, and thus in no logical order. 

Compressed blocks can be data of any type:
* Sequence
* Quality Scores
* Sequence Header / ID
* Sequence Masking

Each block is given a numeric ID (starting at 0), and a location (start and end) in the file. This is later stored in the [block index](../block_index.md)