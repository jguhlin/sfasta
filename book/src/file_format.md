# File Format
The SFASTA format is held together by [bincode](https://github.com/bincode-org/bincode), which serializes structs and other data to the file.

Unlike many other formats, SFASTA is written more similar to a database, and meant to be crawled to properly understand. Linear processing will be difficult, and provide no benefits. Once the directory is read, it is possible to seek to the file location to prase the next bit of data.

It also means if the order of the data changes, the file format remains stable, assuming the directory remains in the same location at the head of the file.

## File Format

| Name | Type | Description |
| ---- | ---- | ----------- |
| Header | str | b"SFASTA" - The "Magic Bytes" |
| Version | u64 | Indicates the version, for future compatability |
| Directory | [Directory](#directory) | Locations of index, sequences, masking, etc... |
| Parameters | [Parameters](#parameters) | Parameters used to create the file |
| Metadata | [Metadata](#metadata) | Metadata about the file |
| Compressed Blocks | [Compressed Blocks](#compressed-blocks) | Individual compressed blocks |
| Block Locations | [Fractal tree](#fractal-tree) | Stored in this order: headers, ids, masking, sequences, scores |
| Headers | [Block Store Headers](#block-store-headers) | Stored in the following order: Headers, IDs, Masking, Sequences, Scores | 

## Directory
| Field | Type | Description |
|:-----:|:----:|:-----------:|
| Index Loc | u64 | Location of the Index Fractal Tree |
| IDs Loc | u64 | Location of the ID Block Store Headers |
| SeqLocs Loc | u64 | Location of the SeqLocs Store Headers |
| Scores Loc | u64 | Location of the quality scores store headers |
| Masking Loc | u64 | Location of the masking store headers |
| Headers loc | u64 | Location of the headers store headers |
| Sequences Loc | u64 | Location of the sequences stores headers |
| Flags Loc | u64 | Not yet implemented |
| Signals Loc | u64 | Not yet implemented |
| Mods Loc | u64 | Not yet implemented |

For all directory entries, 0 indicates None.

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Directory
{
    pub index_loc: Option<NonZeroU64>,
    pub ids_loc: Option<NonZeroU64>,
    pub seqlocs_loc: Option<NonZeroU64>,
    pub scores_loc: Option<NonZeroU64>,
    pub masking_loc: Option<NonZeroU64>,
    pub headers_loc: Option<NonZeroU64>,
    pub sequences_loc: Option<NonZeroU64>,
    pub flags_loc: Option<NonZeroU64>,
    pub signals_loc: Option<NonZeroU64>,
    pub mods_loc: Option<NonZeroU64>,
}
```

## Parameters
| Field | Type | Description |
|:-----:|:----:|:-----------:|
| block size | u32 | Block size used for storage. |

Likely to be removed, and headers to store block size for each individual data type.

```rust
#[derive(Debug, Clone, bincode::Encode, bincode::Decode)]
pub struct Parameters
{
    pub block_size: u32,
}
```

## Metadata
Metadata is stored uncompressed, so that the files can be found with grep and metadata rapidly accessed. These are all open ended, and formats are not enforced.

| Field | Type | Description |
|:-----:|:----:|:-----------:|
| created_by | Option<String> | |
| citation_doi | Option<String> | |
| citation_url | Option<String> | |
| citation_authors | Option<String> | |
| date_created | Option<String> | |
| title | Option<String> | |
| description | Option<String> | |
| notes | Option<String> | |
| download_url | Option<String> | |
| homepage_url | Option<String> | |
| version | Option<String> | |

```rust
#[derive(
    Debug,
    Clone,
    bincode::Encode,
    bincode::Decode,
    Default,
    Serialize,
    Deserialize,
)]
pub struct Metadata
{
    pub created_by: Option<String>,
    pub citation_doi: Option<String>,
    pub citation_url: Option<String>,
    pub citation_authors: Option<String>,
    pub date_created: u64,
    pub title: Option<String>,
    pub description: Option<String>,
    pub notes: Option<String>,
    pub download_url: Option<String>,
    pub homepage_url: Option<String>,
    pub version: Option<usize>,
}
```

## Compressed Blocks
| Field | Type | Description |
|:-----:|:----:|:-----------:|
| data | Vec u8  | |

Data is converted when the block size is reached, or if the file processing is complete. Data is converted to bytes (u8) depending on the store being used (Strings, Masking, etc).

Data is bincoded to Vec u8 , the block is individually compressed, and bincoded into the file as Vec u8.

## Block Store Headers
| Field | Type | Description |
|:-----:|:----:|:-----------:|
| Compression Config | [CompressionConfig](#compression-config) | Configuration for this datatype |
| Location of the Blocks Fractal Tree Index | [FractalTree](#fractal-tree) | Location of the Fractal Tree |
| Block Size | u32 | Size of uncompressed blocks for this data type |

## Compression Config
| Field | Type | Description |
|:-----:|:----:|:-----------:|
| Compression Type | enum | [Compression Type](#compression-type) |
| Compression Level | i8 | Compression level to use for this data type |

```rust
pub struct CompressionConfig
{
    pub compression_type: CompressionType,
    pub compression_level: i8,

    #[serde(skip)]
    pub compression_dict: Option<Vec<u8>>,
}
```

## Compression Type
```rust
#[derive(
    PartialEq,
    Eq,
    Debug,
    Clone,
    Copy,
    bincode::Encode,
    bincode::Decode,
    Serialize,
    Deserialize,
)]
#[non_exhaustive]
pub enum CompressionType
{
    ZSTD,   // 1 should be default compression ratio
    LZ4,    // 9 should be default compression ratio
    SNAPPY, // Implemented
    GZIP,   // Implemented
    NONE,   // No Compression
    XZ,     // Implemented, 6 is default ratio
    BROTLI, // Implemented, 6 is default
    BZIP2,  // Implemented
    BIT2,   // Not implemented
    BIT4,   // Not implemented
}
```

## Fractal Tree
The fractal tree exists in two states, the Build state, and the OnDisk state. It is only stored as on disk.

Fractal Trees are made up of [Nodes](#nodes)

```rust
pub struct FractalTreeDisk<K: Key, V: Value>
{
    pub root: NodeDisk<K, V>,
    pub start: u64, /* On disk position of the fractal tree, such that
                     * all locations are start + offset */
    pub compression: Option<CompressionConfig>,
}
```

### Node
| Field | Type | Description |
|:-----:|:----:|:-----------:|
| is_root | bool | Is the root node? |
| is_leaf | bool | Is a leaf node? |
| state | NodeState | Is the node on the disk, or loaded into memory? |
| keys | Vec(k) | Vector of the keys |
| children | Vec(NodeDisk) | Children of this node (invalid if is_leaf) |
| values | Vec(V) | Values of this leaf node, invalid if is_leaf is false |

```rust
#[derive(Debug, Clone)]
pub struct NodeDisk<K, V>
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub state: NodeState,
    pub keys: Vec<K>,
    pub children: Option<Vec<Box<NodeDisk<K, V>>>>,
    pub values: Option<Vec<V>>,
}
```

### Node State
Represents if the fractal tree node is currently on disk, compressed, or stored in memory (and thus accessible). If stored on disk, contains the location of the data.

```rust
#[derive(Debug, Clone)]
pub enum NodeState
{
    InMemory,
    Compressed(Vec<u8>),
    OnDisk(u32),
}
```