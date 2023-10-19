# File Format

## File Format Table

| Name | Type | Description |
| ---- | ---- | ----------- |
| Header | str | b"SFASTA" |
| Version | u64 | Indicated the version, for future compatability |
| Directory | Struct: [Directory](./structs/directory.md) | Locations of index, sequences, masking, etc... |
| Parameters | Struct: [Parameters](./structs/parameters.md) | Parameters used to create the file |
| Metadata | Struct: [Metadata](./structs/metadata.md) | Metadata about the file |
| Compressed Blocks | [Compressed Blocks](./datatypes/compressed_blocks.md) | Compressed blocks of data |
| Block Index | [Block Index](./block_index.md) | Index of the compressed blocks |

