# Block Index

## Format
| Name | Type | Description |
| ---- | ---- | ----------- |
| Configuration | [Configuration Struct](#configuration) | Configuration to read this index |
| Block Indexes | Vec\<u64\> | Compressed Block Locations, stored as Stream Vbyte(?) |
| Block Locations | Vec\<u32\> | Locations of each block, offset from the end of the configuration (thus, loc of next byte after config + u32) |

## Configuration
| Name | Type | Description |
| ---- | ---- | ----------- |
| Quantity | u8 | Number of total block indexes, as each only supports u32 addressing |
| Locations Per Block | u16 | Number of blocks locations per index block |
| Next Block | u64 | Location of the next block index, if applicable |
| Block Index | u32 | Location of the indices for this block (at the end, after all blocks). Location is end of configuration + this u32 |

## Over u32::MAX
If the file is over u32::MAX bytes, the block index will be split into multiple parts. The first part will be stored at the end of the file, and the next part will be stored at the location specified by the `Next Block` field in the configuration. Each block will have a configuration, and the last block will have a `Next Block` of 0.

Likely not implemented yet.