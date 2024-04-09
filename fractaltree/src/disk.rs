use std::io::{BufRead, Read, Seek, SeekFrom, Write};

use bincode::{BorrowDecode, Decode, Encode};
use binout::{Serializer, VByte};
use pulp::Arch;

use crate::*;
use libcompression::*;

// Tried to make it generic, didn't work
// Maybe try again later...

/// This is the on-disk version of the FractalTree
///
/// The root node is loaded with the fractal tree, but children are loaded on demand
#[derive(Debug)]
pub struct FractalTreeDisk<K: Key, V: Value>
{
    pub root: NodeDisk<K, V>,
    pub start: u64, /* On disk position of the fractal tree, such that
                     * all locations are start + offset */
    pub compression: Option<CompressionConfig>,
}

impl<K: Key, V: Value> Encode for FractalTreeDisk<K, V>
{
    fn encode<E: bincode::enc::Encoder>(&self, encoder: &mut E)
        -> core::result::Result<(), bincode::error::EncodeError>
    {
        bincode::Encode::encode(&self.compression, encoder)?;
        bincode::Encode::encode(&self.start, encoder)?;
        if self.compression.is_some() {
            let encoded = bincode::encode_to_vec(&self.root, *encoder.config()).unwrap();
            let data = self.compression.as_ref().unwrap().compress(&encoded).unwrap();
            bincode::Encode::encode(&data, encoder)
        } else {
            bincode::Encode::encode(&self.root, encoder)
        }
    }
}

impl<K: Key, V: Value> Decode for FractalTreeDisk<K, V>
{
    fn decode<D: bincode::de::Decoder>(decoder: &mut D) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let compression: Option<CompressionConfig> = bincode::Decode::decode(decoder)?;
        let start: u64 = bincode::Decode::decode(decoder)?;
        let root: NodeDisk<K, V> = if compression.is_some() {
            let compressed: Vec<u8> = bincode::Decode::decode(decoder).unwrap();
            let decompressed = compression.as_ref().unwrap().decompress(&compressed).unwrap();
            bincode::decode_from_slice(&decompressed, *decoder.config()).unwrap().0
        } else {
            bincode::Decode::decode(decoder)?
        };
        Ok(FractalTreeDisk {
            root,
            start,
            compression,
        })
    }
}

impl<K: Key, V: Value> Default for FractalTreeDisk<K, V>
{
    fn default() -> Self
    {
        FractalTreeDisk {
            root: NodeDisk {
                is_root: true,
                state: None,
                is_leaf: true,
                keys: Vec::new(),
                children: None,
                values: None,
            },
            start: 0,
            compression: None, // Default to no compression
        }
    }
}

impl<K: Key, V: Value> FractalTreeDisk<K, V>
{
    pub fn load_tree<R>(&mut self, in_buf: &mut R) -> Result<(), &'static str>
    where
        R: Read + Seek + BufRead,
    {
        self.root.load_all(in_buf, &self.compression, self.start);
        Ok(())
    }

    pub fn len(&self) -> Result<usize, &'static str>
    {
        self.root.len()
    }

    pub fn set_compression(&mut self, compression: CompressionConfig)
    {
        self.compression = Some(compression);
    }

    pub fn search<R>(&mut self, in_buf: &mut R, key: &K) -> Option<&V>
    where
        R: Read + Seek + Send + Sync,
    {
        self.root.is_root = true; // TODO: Move this to custom decode
        self.root.search(in_buf, &self.compression, self.start, key)
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: W) -> Result<u64, &'static str>
    where
        W: Write + Seek,
    {
        let start_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
        self.start = start_pos;

        self.root.store(&mut out_buf, &self.compression, start_pos);

        let tree_location = out_buf.seek(SeekFrom::Current(0)).unwrap();
        let bincode_config = bincode::config::standard().with_variable_int_encoding();
        bincode::encode_into_std_write(&*self, &mut out_buf, bincode_config).unwrap();

        log::debug!("Start Pos: {}", self.start);

        Ok(tree_location)
    }

    pub fn from_buffer<R>(mut in_buf: R, pos: u64) -> Result<Self, &'static str>
    where
        R: Read + Seek + BufRead,
    {
        in_buf.seek(SeekFrom::Start(pos)).unwrap();
        let bincode_config = bincode::config::standard().with_variable_int_encoding();

        let tree: FractalTreeDisk<K, V> = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        log::debug!("Start Pos: {}", tree.start);
        log::debug!("Root Children Count: {:?}", tree.root.children.as_ref().unwrap().len());
        log::debug!("Root Keys: {:?}", tree.root.keys);

        Ok(tree)
    }
}

#[derive(Debug)]
pub struct NodeDisk<K, V>
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub state: Option<u32>, // None means in memory, Some(u32) is the location on the disk
    pub keys: Vec<K>,
    pub children: Option<Vec<Box<NodeDisk<K, V>>>>,
    pub values: Option<Vec<V>>,
}

impl<K: Key, V: Value> Encode for NodeDisk<K, V>
{
    fn encode<E: bincode::enc::Encoder>(&self, encoder: &mut E)
        -> core::result::Result<(), bincode::error::EncodeError>
    {
        bincode::Encode::encode(&self.is_leaf, encoder)?;
        bincode::Encode::encode(&self.keys, encoder)?;

        if self.is_leaf {
            bincode::Encode::encode(&self.values, encoder)?;
        } else {
            let locs = self
                .children
                .as_ref()
                .unwrap()
                .iter()
                .map(|x| x.state.as_ref().unwrap().to_owned())
                .collect::<Vec<u32>>();

            bincode::Encode::encode(&locs, encoder)?;
        }
        Ok(())
    }
}

impl<K: Key, V: Value> Decode for NodeDisk<K, V>
{
    fn decode<D: bincode::de::Decoder>(decoder: &mut D) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let is_leaf: bool = bincode::Decode::decode(decoder)?;
        let keys: Vec<K> = bincode::Decode::decode(decoder)?;

        let values: Option<Vec<V>> = if is_leaf {
            bincode::Decode::decode(decoder)?
        } else {
            None
        };

        let children = if is_leaf {
            None
        } else {
            let locs: Vec<u32> = bincode::Decode::decode(decoder)?;
            let children: Vec<Box<NodeDisk<K, V>>> = locs
                .iter()
                .map(|x| Box::new(NodeDisk::<K, V>::from_loc(*x)))
                .collect::<Vec<_>>();
            Some(children)
        };

        Ok(NodeDisk {
            is_root: false,
            is_leaf,
            state: None,
            keys: keys.to_vec(),
            children,
            values,
        })
    }
}

impl<K: Key, V: Value> BorrowDecode<'_> for NodeDisk<K, V>
{
    fn borrow_decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let is_leaf: bool = bincode::Decode::decode(decoder)?;
        let keys: Vec<K> = bincode::Decode::decode(decoder)?;

        let values: Option<Vec<V>> = if is_leaf {
            bincode::Decode::decode(decoder)?
        } else {
            None
        };

        let children = if is_leaf {
            None
        } else {
            let locs: Vec<u32> = bincode::Decode::decode(decoder)?;
            let children: Vec<Box<NodeDisk<K, V>>> = locs
                .iter()
                .map(|x| Box::new(NodeDisk::<K, V>::from_loc(*x)))
                .collect::<Vec<_>>();
            Some(children)
        };

        Ok(NodeDisk {
            is_root: false,
            is_leaf,
            state: None,
            keys: keys.to_vec(),
            children,
            values,
        })
    }
}

impl<K: Key, V: Value> NodeDisk<K, V>
{
    pub fn from_loc(loc: u32) -> Self
    {
        NodeDisk {
            is_root: false,
            is_leaf: false,
            state: Some(loc),
            keys: Vec::new(),
            children: None,
            values: None,
        }
    }

    pub fn load<R>(&mut self, in_buf: &mut R, compression: &Option<CompressionConfig>, start: u64)
    where
        R: Read + Seek,
    {
        log::debug!("Start {} State {:?}", start, self.state);
        in_buf
            .seek(SeekFrom::Start(start + self.state.unwrap() as u64))
            .unwrap();

        *self = if compression.is_some() {
            let config = bincode::config::standard()
                .with_fixed_int_encoding()
                .with_limit::<{ 1024 * 1024 }>();

            let compressed: Vec<u8> = bincode::decode_from_std_read(in_buf, config).unwrap();
            let decompressed = compression.as_ref().unwrap().decompress(&compressed).unwrap();
            bincode::decode_from_slice(&decompressed, config).unwrap().0
        } else {
            let config = bincode::config::standard()
                .with_variable_int_encoding()
                .with_limit::<{ 1024 * 1024 }>();
            bincode::decode_from_std_read(in_buf, config).unwrap()
        };
    }

    // todo: This doesn't work, need to account for compression better
    pub fn load_all<R>(&mut self, in_buf: &mut R, compression: &Option<CompressionConfig>, start: u64)
    where
        R: Read + Seek,
    {
        let config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<{ 1024 * 1024 }>();

        if self.state.is_some() {
            in_buf
                .seek(SeekFrom::Start(self.state.unwrap() as u64 + start))
                .unwrap();
            let node: NodeDisk<K, V> = bincode::decode_from_std_read(in_buf, config).unwrap();
            *self = node;
        }

        if !self.is_leaf {
            for child in self.children.as_mut().unwrap() {
                child.load_all(in_buf, &compression, start);
            }
        }

        self.state = None;
    }

    pub fn store<W>(&mut self, out_buf: &mut W, compression: &Option<CompressionConfig>, start: u64)
    where
        W: Write + Seek,
    {
        // Make sure all the children are stored first...
        if !self.is_leaf {
            for child in self.children.as_mut().unwrap() {
                if !child.state.is_some() {
                    child.store(out_buf, compression, start);
                }
            }
        }

        let cur_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
        // Don't store the root separately...
        if !self.is_root {
            if compression.is_some() {
                let config = bincode::config::standard()
                    .with_fixed_int_encoding()
                    .with_limit::<{ 1024 * 1024 }>();

                let uncompressed: Vec<u8> = bincode::encode_to_vec(&*self, config).unwrap();
                let compressed = compression.as_ref().unwrap().compress(&uncompressed).unwrap();
                bincode::encode_into_std_write(&compressed, out_buf, config).unwrap();

                self.state = Some(cur_pos as u32 - start as u32);
                self.children = None;
                self.values = None;
            } else {
                let config = bincode::config::standard()
                    .with_variable_int_encoding()
                    .with_limit::<{ 1024 * 1024 }>();
                bincode::encode_into_std_write(&*self, out_buf, config).unwrap();

                self.state = Some(cur_pos as u32 - start as u32);
                self.children = None;
                self.values = None;
            }
        }
    }

    pub fn search<R>(
        &mut self,
        in_buf: &mut R,
        compression: &Option<CompressionConfig>,
        start: u64,
        key: &K,
    ) -> Option<&V>
    where
        R: Read + Seek + Send + Sync,
    {
        if self.state.is_some() {
            self.load(in_buf, compression, start);
        }

        if self.is_leaf {
            let i = self.keys.binary_search(&key);
            let i = match i {
                Ok(i) => i,
                Err(_) => return None,
            };

            Some(&self.values.as_ref().unwrap()[i])
        } else {
            let i = self.keys.binary_search(&key);
            let i = match i {
                Ok(i) => i.saturating_add(1),
                Err(i) => i,
            };

            self.children.as_mut().unwrap()[i].search(in_buf, compression, start, key)
        }
    }

    pub fn children_stored_on_disk(&self) -> bool
    {
        if self.is_leaf {
            true
        } else {
            self.children.as_ref().unwrap().iter().all(|x| !x.state.is_some())
        }
    }

    pub fn len(&self) -> Result<usize, &'static str>
    {
        if self.is_leaf {
            Ok(self.keys.len())
        } else {
            let mut len = 0;
            if self.children.is_none() {
                return Err("Children not loaded");
            }

            for i in 0..self.children.as_ref().unwrap().len() {
                len += self.children.as_ref().unwrap()[i].len()?;
            }

            Ok(len)
        }
    }
}

#[cfg(test)]
mod tests
{
    use super::*;
    use crate::*;
    use human_size::SpecificSize;
    use rand::prelude::*;

    #[test]
    fn node_storage()
    {
        let rng = thread_rng();
        // Get 128 random values
        let values: Vec<u32> = rng.sample_iter(rand::distributions::Standard).take(128).collect();

        let rng = thread_rng();
        let mut keys: Vec<u32> = rng.sample_iter(rand::distributions::Standard).take(128).collect();
        keys.sort();

        let mut node = NodeDisk {
            is_root: true,
            is_leaf: true,
            state: None,
            keys,
            children: None,
            values: Some(values),
        };

        let bincode_config = bincode::config::standard().with_variable_int_encoding();

        let mut buf = std::io::BufWriter::new(std::io::Cursor::new(Vec::new()));
        bincode::encode_into_std_write(&node.is_leaf, &mut buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&node.keys, &mut buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&node.values.as_ref().unwrap(), &mut buf, bincode_config).unwrap();
        let no_vbyte_len = buf.into_inner().unwrap().into_inner().len();

        let mut buf = std::io::BufWriter::new(std::io::Cursor::new(Vec::new()));
        node.store(&mut buf, &None, 0);

        let len = buf.into_inner().unwrap().into_inner().len();
        let size: SpecificSize<human_size::Kilobyte> = format!("{} B", len).parse().unwrap();

        println!("Node size: {}", size);

        let size: SpecificSize<human_size::Kilobyte> = format!("{} B", no_vbyte_len).parse().unwrap();
        println!("Node size (no vbyte): {}", size);
    }

    #[test]
    fn tree_storage()
    {
        let mut rng = thread_rng();

        let mut tree = FractalTreeBuild::new(128, 256);

        // Generate 1024 * 1024 random key value pairs

        for _ in 0..1024 * 1024 {
            let key = rng.gen::<u32>();
            let value = rng.gen::<u32>();

            tree.insert(key, value);
        }

        // Guaranteed insert to try and find later
        tree.insert(1, 1);
        tree.insert(u32::MAX - 1, u32::MAX - 1);
        tree.insert(u32::MAX / 2, u32::MAX / 2);

        let mut tree: FractalTreeDisk<u32, u32> = tree.into();
        let orig_root_keys = tree.root.keys.clone();
        println!("Root keys: {:?}", orig_root_keys);

        let mut buf = std::io::BufWriter::new(std::io::Cursor::new(Vec::new()));
        let tree_loc = tree.write_to_buffer(&mut buf).unwrap();

        // --------------------------------------------
        // Load up the tree now

        let raw_vec = buf.into_inner().unwrap().into_inner();

        println!("Raw Vec Len: {}", raw_vec.len());

        let mut buf = std::io::BufReader::new(std::io::Cursor::new(raw_vec));

        buf.seek(SeekFrom::Start(tree_loc)).unwrap();
        let mut tree: FractalTreeDisk<u32, u32> =
            bincode::decode_from_std_read(&mut buf, bincode::config::standard().with_variable_int_encoding()).unwrap();
        assert!(tree.root.keys == orig_root_keys);

        let result = tree.search(&mut buf, &1);
        assert!(result.is_some());
        assert!(*result.unwrap() == 1);

        let result = tree.search(&mut buf, &(u32::MAX - 1));
        assert!(result.is_some());
        assert!(*result.unwrap() == u32::MAX - 1);

        let result = tree.search(&mut buf, &(u32::MAX / 2));
        assert!(result.is_some());
        assert!(*result.unwrap() == u32::MAX / 2);
    }

    #[ignore]
    #[test]
    /// This is mostly to see how large a very large tree is (in gigs, and megabytes)
    fn tree_large_storage()
    {
        let mut rng = thread_rng();

        let mut tree = FractalTreeBuild::new(128, 256);

        // Generate 1024 * 1024 random key value pairs

        for _ in 0..160 * 1024 * 1024 {
            let key = rng.gen::<u32>();
            let value = rng.gen::<u32>();

            tree.insert(key, value);
        }

        // Guaranteed insert to try and find later
        tree.insert(1, 1);
        tree.insert(u32::MAX - 1, u32::MAX - 1);
        tree.insert(u32::MAX / 2, u32::MAX / 2);

        let mut tree: FractalTreeDisk<u32, u32> = tree.into();
        let orig_root_keys = tree.root.keys.clone();

        let mut buf = std::io::BufWriter::new(std::io::Cursor::new(Vec::new()));
        let tree_loc = tree.write_to_buffer(&mut buf).unwrap();

        // --------------------------------------------
        // Load up the tree now

        let raw_vec = buf.into_inner().unwrap().into_inner();

        println!("Raw Vec Len: {}", raw_vec.len());
        let size2: SpecificSize<human_size::multiples::Gigabyte> = format!("{} B", raw_vec.len()).parse().unwrap();
        println!("Raw Vec Size: {}", size2);
        let size2: SpecificSize<human_size::multiples::Megabyte> = format!("{} B", raw_vec.len()).parse().unwrap();
        println!("Raw Vec Size: {}", size2);

        let mut buf = std::io::BufReader::new(std::io::Cursor::new(raw_vec));

        buf.seek(SeekFrom::Start(tree_loc)).unwrap();
        let mut tree: FractalTreeDisk<u32, u32> =
            bincode::decode_from_std_read(&mut buf, bincode::config::standard().with_variable_int_encoding()).unwrap();
        assert!(tree.root.keys == orig_root_keys);

        let result = tree.search(&mut buf, &1);
        assert!(result.is_some());
        assert!(*result.unwrap() == 1);

        let result = tree.search(&mut buf, &(u32::MAX - 1));
        assert!(result.is_some());
        assert!(*result.unwrap() == u32::MAX - 1);

        let result = tree.search(&mut buf, &(u32::MAX / 2));
        assert!(result.is_some());
        assert!(*result.unwrap() == u32::MAX / 2);

        panic!();
    }
}
