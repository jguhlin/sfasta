use std::{
    io::{BufRead, Read, Seek, SeekFrom, Write},
    ops::SubAssign,
};

use bincode::{BorrowDecode, Decode, Encode};
use pulp::Arch;

use crate::*;
use libcompression::*;

// todo
// todo: load_all use decompression
// todo: in bytes block store and seqlocs make this into a different
// thread (not a here todo, but elsewhere) todo: speed would come here
// from batch inserts! todo: test out simd search instead of binary
// search (use pulp arch)

/// This is the on-disk version of the FractalTree
///
/// The root node is loaded with the fractal tree; children are
/// loaded on demand
#[derive(Debug, Clone)]
pub struct FractalTreeDisk<K: Key, V: Value>
{
    pub root: NodeDisk<K, V>,
    pub compression: Option<CompressionConfig>,
    pub start: u64, /* On disk position of the fractal tree, such that
                     * all locations are start + offset */
}

impl<K: Key, V: Value> Encode for FractalTreeDisk<K, V>
{
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), bincode::error::EncodeError>
    {
        bincode::Encode::encode(&self.start, encoder)?;
        bincode::Encode::encode(&self.compression, encoder)?;
        if self.compression.is_some() {
            let encoded =
                bincode::encode_to_vec(&self.root, *encoder.config()).unwrap();
            let data = self
                .compression
                .as_ref()
                .unwrap()
                .compress(&encoded)
                .unwrap();
            bincode::Encode::encode(&data, encoder)
        } else {
            bincode::Encode::encode(&self.root, encoder)
        }
    }
}

impl<K: Key, V: Value> Decode for FractalTreeDisk<K, V>
{
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let start: u64 = bincode::Decode::decode(decoder)?;
        // let first_leaf: u32 = bincode::Decode::decode(decoder)?;
        let compression: Option<CompressionConfig> =
            bincode::Decode::decode(decoder)?;
        let root: NodeDisk<K, V> = if compression.is_some() {
            let compressed: Vec<u8> = bincode::Decode::decode(decoder)?;
            let decompressed = compression
                .as_ref()
                .unwrap()
                .decompress(&compressed)
                .unwrap();
            bincode::decode_from_slice(&decompressed, *decoder.config())
                .unwrap()
                .0
        } else {
            bincode::Decode::decode(decoder)?
        };
        Ok(FractalTreeDisk {
            root,
            start,
            compression,
            // first_leaf,
        })
    }
}

impl<K: Key, V: Value> Default for FractalTreeDisk<K, V>
{
    fn default() -> Self
    {
        FractalTreeDisk {
            root: NodeDisk {
                ..Default::default()
            },
            start: 0,
            compression: None, // No compression by default
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

    pub fn create_zstd_dict(&mut self) -> Vec<u8>
    {
        let out_buf = Vec::new();
        let mut out_buf = std::io::Cursor::new(out_buf);

        let mut sample_sizes = Vec::new();

        let mut new_root = self.root.clone();
        new_root.store_dummy(&mut out_buf, &mut sample_sizes);

        log::info!(
            "Average Size: {} Min/Max: {}/{}",
            sample_sizes.iter().sum::<usize>() / sample_sizes.len(),
            sample_sizes.iter().min().unwrap(),
            sample_sizes.iter().max().unwrap()
        );

        log::debug!(
            "Number of samples: {} Max Dict Size: {}",
            sample_sizes.len(),
            32 * 1024
        );

        let buf = out_buf.into_inner();

        match zstd::dict::from_continuous(&buf, &sample_sizes, 32 * 1024) {
            Ok(dict) => dict,
            Err(e) => {
                panic!("Error creating zstd dict: {:?}", e);
            }
        }
    }

    pub fn search<R>(&mut self, in_buf: &mut R, key: &K) -> Option<V>
    where
        R: Read + Seek + Send + Sync,
    {
        self.root.is_root = true; // TODO: Move this to custom decode
        self.root.search(in_buf, &self.compression, self.start, key)
    }

    pub fn write_to_buffer<W>(
        &mut self,
        mut out_buf: W,
    ) -> Result<u64, &'static str>
    where
        W: Write + Seek,
    {
        let start_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
        self.start = start_pos;

        // self.root.store(&mut out_buf, &self.compression, start_pos);
        self.store_by_layer(&mut out_buf, start_pos)?;

        let tree_location = out_buf.seek(SeekFrom::Current(0)).unwrap();
        let bincode_config =
            bincode::config::standard().with_fixed_int_encoding();
        bincode::encode_into_std_write(&*self, &mut out_buf, bincode_config)
            .unwrap();

        Ok(tree_location)
    }

    pub fn store_by_layer<W>(
        &mut self,
        mut out_buf: W,
        start: u64, // Start position of the tree being stored
    ) -> Result<(), &'static str>
    where
        W: Write + Seek,
    {
        // Store the leaves first, then come back up
        // Root node is stored with the tree struct
        // Given the layer number, go down to that layer and begin storing

        // Not writing the root here... but if the root is as deep as we go,
        // we are done...
        if self.root.is_leaf {
            log::debug!("Tree root is a leaf, nothing to store");
            return Ok(());
        }

        // (layer, node)
        let mut working_stack = Vec::new();

        // (layer, node)
        let mut accounted_for = Vec::new();

        // todo paths can be one vec and then stored as slices
        // probs not worth the effort tho

        for (i, _child) in
            self.root.children.as_ref().unwrap().into_iter().enumerate()
        {
            working_stack.push((1, vec![i]));
        }

        while !working_stack.is_empty() {
            let (layer, path) = working_stack.pop().unwrap();

            // Get node via path
            let mut node = &self.root.children.as_ref().unwrap()[path[0]];
            for i in &path[1..] {
                assert!(!node.is_leaf);
                node = &node.children.as_ref().unwrap()[*i];
            }

            if !node.is_leaf {
                // Otherwise, get the children and add them to the working stack
                for (i, _child) in
                    node.children.as_ref().unwrap().into_iter().enumerate()
                {
                    let mut new_path = path.clone();
                    new_path.push(i);
                    working_stack.push((layer + 1, new_path));
                }
            }

            accounted_for.push((layer, path));
        }

        // Ok, now we have (convoluted) which layer, and the path to get to
        // each node, let's start storing them, left to right, leaves first
        // That way we can decompress the leaves first, and stop when is_leaf
        // == false if we want to do a linear search... This beats the
        // borrow checker

        // Get the max layer number
        let max_layer = accounted_for.iter().map(|x| x.0).max().unwrap();
        log::debug!(
            "Tree has {} layers - Compression with {:?}",
            max_layer,
            self.compression.as_ref().unwrap().compression_type
        );

        for layer in (1..=max_layer).rev() {
            // Get all the nodes at this layer
            let nodes = accounted_for.iter().filter(|x| x.0 == layer);

            // Debugging stuff
            // Make sure the keys are ordered properly for leaves...
            let mut nodes: Vec<&(i32, Vec<usize>)> = nodes.collect();
            // i32 is the layer, Vec<usize> is the path to the node

            // log::trace!("Storing layer {} - {} nodes", layer, nodes.len());

            // Sort by the first key, lowest to highest
            nodes.sort_by(|&a, &b| {
                let mut node_a = &self.root.children.as_ref().unwrap()[a.1[0]];

                for i in &a.1[1..] {
                    node_a = &node_a.children.as_ref().unwrap()[*i];
                }

                let mut node_b = &self.root.children.as_ref().unwrap()[b.1[0]];

                for i in &b.1[1..] {
                    node_b = &node_b.children.as_ref().unwrap()[*i];
                }

                node_a.keys[0].cmp(&node_b.keys[0])
            });

            let mut previous_key = K::default();

            // todo can we compress children in parallel?

            for (_layer, path) in nodes {
                let mut node =
                    &mut self.root.children.as_mut().unwrap()[path[0]];

                for i in &path[1..] {
                    assert!(!node.is_leaf);
                    node = &mut node.children.as_mut().unwrap()[*i];
                }

                if !node.is_leaf {
                    assert!(node.children_stored_on_disk());
                }

                assert!(node.keys[0] >= previous_key);
                previous_key = node.keys[node.keys.len() - 1];

                node.store(&mut out_buf, &self.compression, start);
            }
        }
        Ok(())
    }

    pub fn from_buffer<R>(mut in_buf: R, pos: u64) -> Result<Self, &'static str>
    where
        R: Read + Seek + BufRead,
    {
        in_buf.seek(SeekFrom::Start(pos)).unwrap();
        let bincode_config =
            bincode::config::standard().with_fixed_int_encoding();

        let tree: FractalTreeDisk<K, V> =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();

        Ok(tree)
    }
}

#[derive(Debug, Clone, Default)]
pub enum NodeState
{
    #[default]
    InMemory,
    Compressed(Vec<u8>),
    OnDisk(u32),
}

impl NodeState
{
    pub fn as_ref(&self) -> Option<u32>
    {
        match self {
            NodeState::OnDisk(x) => Some(*x),
            _ => None,
        }
    }

    pub fn on_disk(&self) -> bool
    {
        match self {
            NodeState::OnDisk(_) => true,
            _ => false,
        }
    }

    pub fn compressed(&self) -> bool
    {
        match self {
            NodeState::Compressed(_) => true,
            _ => false,
        }
    }

    pub fn in_memory(&self) -> bool
    {
        match self {
            NodeState::InMemory => true,
            _ => false,
        }
    }

    pub fn loc(&self) -> u32
    {
        match self {
            NodeState::OnDisk(x) => *x,
            _ => panic!("Node not on disk"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeDisk<K, V>
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub state: NodeState,
    pub keys: Vec<K>,
    pub children: Option<Vec<Box<NodeDisk<K, V>>>>,
    pub values: Option<Vec<V>>,
}

impl<K: Key, V: Value> Encode for NodeDisk<K, V>
{
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), bincode::error::EncodeError>
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
                .map(|x| x.state.as_ref().unwrap())
                .collect::<Vec<u32>>();

            bincode::Encode::encode(&locs, encoder)?;
        }

        // debug_assert!(self.right.is_none() || self.is_leaf &&
        // self.right.is_some());

        // if self.is_leaf {
        // bincode::Encode::encode(&self.right.unwrap(), encoder)?;
        // }

        Ok(())
    }
}

impl<K: Key, V: Value> Decode for NodeDisk<K, V>
{
    fn decode<D: bincode::de::Decoder>(
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

        // let right = if is_leaf {
        // let right: u32 = bincode::Decode::decode(decoder)?;
        // Some(right)
        // } else {
        // None
        // };

        Ok(NodeDisk {
            is_root: false,
            is_leaf,
            state: NodeState::InMemory,
            keys,
            children,
            values,
            // right,
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

        // let right = if is_leaf {
        // let right: u32 = bincode::Decode::decode(decoder)?;
        // Some(right)
        // } else {
        // None
        // };

        Ok(NodeDisk {
            is_root: false,
            is_leaf,
            state: NodeState::InMemory,
            keys: keys.to_vec(),
            children,
            values,
            // right,
        })
    }
}

impl<K: Key, V: Value> NodeDisk<K, V>
{
    pub fn from_loc(loc: u32) -> Self
    {
        NodeDisk {
            state: NodeState::OnDisk(loc),
            ..Default::default()
        }
    }

    pub fn load<R>(
        &mut self,
        in_buf: &mut R,
        compression: &Option<CompressionConfig>,
        start: u64,
    ) where
        R: Read + Seek,
    {
        in_buf
            .seek(SeekFrom::Start(start + self.state.loc() as u64))
            .unwrap();

        *self = if compression.is_some() {
            let config = bincode::config::standard()
                .with_fixed_int_encoding()
                .with_limit::<{ 8 * 1024 * 1024 }>();

            let compressed: Vec<u8> =
                bincode::decode_from_std_read(in_buf, config).unwrap();
            let decompressed = compression
                .as_ref()
                .unwrap()
                .decompress(&compressed)
                .unwrap();
            bincode::decode_from_slice(&decompressed, config).unwrap().0
        } else {
            let config = bincode::config::standard()
                .with_fixed_int_encoding()
                .with_limit::<{ 1024 * 1024 }>();
            bincode::decode_from_std_read(in_buf, config).unwrap()
        };

        delta_decode(&mut self.keys);
    }

    // todo: This doesn't work, need to account for compression better
    pub fn load_all<R>(
        &mut self,
        in_buf: &mut R,
        compression: &Option<CompressionConfig>,
        start: u64,
    ) where
        R: Read + Seek,
    {
        if self.state.on_disk() {
            self.load(in_buf, compression, start);
        }

        if !self.is_leaf {
            for child in self.children.as_mut().unwrap() {
                child.load_all(in_buf, &compression, start);
            }
        }

        self.state = NodeState::InMemory;
    }

    /// Stores the node to the buffer, and returns the size of the
    /// node. Useful for creating dictionaries...
    pub fn store_dummy<W>(
        &mut self,
        out_buf: &mut W,
        sample_sizes: &mut Vec<usize>,
    ) where
        W: Write + Seek,
    {
        let config = bincode::config::standard()
            .with_fixed_int_encoding()
            .with_limit::<{ 1024 * 1024 }>();

        // let mut new_self = self.clone();

        // Make sure all the children are stored first...
        if !self.is_leaf {
            for child in self.children.as_mut().unwrap() {
                while !child.state.on_disk() {
                    child.store_dummy(out_buf, sample_sizes);
                }
            }
        }

        let cur_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
        // Don't store the root separately...
        if !self.is_root {
            delta_encode(&mut self.keys);

            match bincode::encode_into_std_write(&*self, out_buf, config) {
                Ok(size) => {
                    sample_sizes.push(size);
                }
                Err(e) => {
                    panic!("Error bincoding NodeDisk: {:?}", e)
                }
            }

            self.state = NodeState::OnDisk(cur_pos as u32);
            self.children = None;
            self.values = None;
        }
    }

    pub fn store<W>(
        &mut self,
        out_buf: &mut W,
        compression: &Option<CompressionConfig>,
        start: u64,
    ) where
        W: Write + Seek,
    {
        // Make sure all the children are stored first...
        if !self.is_leaf {
            for child in self.children.as_mut().unwrap() {
                if !child.state.on_disk() {
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

                /*
                log::trace!(
                    "Node has {} keys: {} {}",
                    self.keys.len(),
                    self.is_leaf,
                    self.is_root,
                );

                // First 5 keys are:
                log::trace!("Keys: {:?}", &self.keys[..5]);

                // todo remove
                assert!(self.keys.len() <= 256);

                */

                delta_encode(&mut self.keys);
                let uncompressed: Vec<u8> =
                    bincode::encode_to_vec(&*self, config).unwrap();

                let compressed = compression
                    .as_ref()
                    .unwrap()
                    .compress(&uncompressed)
                    .unwrap();

                match bincode::encode_into_std_write(
                    &compressed,
                    out_buf,
                    config,
                ) {
                    Ok(size) => (), // log::debug!("Compressed size of
                    // NodeDisk with {:?}: {}",
                    // compression.as_ref().unwrap().
                    // compression_type, size),
                    Err(e) => {
                        panic!("Error compressing NodeDisk: {:?}", e)
                    }
                }

                self.state = NodeState::OnDisk(cur_pos as u32 - start as u32);
                self.children = None;
                self.values = None;

            } else {

                delta_encode(&mut self.keys);
                let config = bincode::config::standard()
                    .with_fixed_int_encoding()
                    .with_limit::<{ 1024 * 1024 }>();
                bincode::encode_into_std_write(&*self, out_buf, config)
                    .unwrap();

                self.state = NodeState::OnDisk(cur_pos as u32 - start as u32);
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
    ) -> Option<V>
    where
        R: Read + Seek + Send + Sync,
    {
        if self.state.on_disk() {
            self.load(in_buf, compression, start);
        }

        // Optimization notes:
        // Binary search is a bit faster than linear search (even in --release
        // mode) and equal to using pulp's automatic dispatch
        let i = self.keys.binary_search(&key);

        if self.is_leaf {
            let i = match i {
                Ok(i) => i,
                Err(_) => return None,
            };

            Some(self.values.as_ref().unwrap()[i].clone())
        } else {
            let i = match i {
                Ok(i) => i.saturating_add(1),
                Err(i) => i,
            };

            self.children.as_mut().unwrap()[i].search(
                in_buf,
                compression,
                start,
                key,
            )
        }
    }

    pub fn children_stored_on_disk(&self) -> bool
    {
        if self.is_leaf {
            true
        } else {
            self.children
                .as_ref()
                .unwrap()
                .iter()
                .all(|x| x.state.on_disk())
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

// it works! trying this now
// todo try pulp
// todo try wide crate?
// or vers vers-vecs Elias-Fano
pub fn delta_encode<T>(values: &mut [T])
where
    T: Default + num::traits::Unsigned + Copy + SubAssign,
{
    let arch = Arch::new();

    arch.dispatch(|| {
        let mut prev: T = Default::default();
        for i in values {
            let tmp = *i;
            *i -= prev;
            prev = tmp;
        }
    });
}

pub fn delta_decode<T>(values: &mut [T])
where
    T: Default + num::traits::Unsigned + Copy + AddAssign,
{
    let arch = Arch::new();

    arch.dispatch(|| {
        let mut prev: T = Default::default();
        for i in values {
            *i += prev;
            prev = *i;
        }
    });
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
        let values: Vec<u32> = rng
            .sample_iter(rand::distributions::Standard)
            .take(128)
            .collect();

        let rng = thread_rng();
        let mut keys: Vec<u32> = rng
            .sample_iter(rand::distributions::Standard)
            .take(128)
            .collect();
        keys.sort();

        let mut node = NodeDisk {
            is_root: true,
            is_leaf: true,
            state: NodeState::InMemory,
            keys,
            children: None,
            values: Some(values),
        };

        let bincode_config =
            bincode::config::standard().with_fixed_int_encoding();

        let mut buf = std::io::BufWriter::new(std::io::Cursor::new(Vec::new()));
        bincode::encode_into_std_write(&node.is_leaf, &mut buf, bincode_config)
            .unwrap();
        bincode::encode_into_std_write(&node.keys, &mut buf, bincode_config)
            .unwrap();
        bincode::encode_into_std_write(
            &node.values.as_ref().unwrap(),
            &mut buf,
            bincode_config,
        )
        .unwrap();
        let no_vbyte_len = buf.into_inner().unwrap().into_inner().len();

        let mut buf = std::io::BufWriter::new(std::io::Cursor::new(Vec::new()));
        node.store(&mut buf, &None, 0);

        let len = buf.into_inner().unwrap().into_inner().len();
        let size: SpecificSize<human_size::Kilobyte> =
            format!("{} B", len).parse().unwrap();

        println!("Node size: {}", size);

        let size: SpecificSize<human_size::Kilobyte> =
            format!("{} B", no_vbyte_len).parse().unwrap();
        println!("Node size (no vbyte): {}", size);
    }

    #[test]
    fn tree_create_dict()
    {
        let mut rng = thread_rng();

        let mut tree = FractalTreeBuild::new(256, 8192);

        // Generate 1024 * 1024 random key value pairs

        for i in 0..1024 * 1024_u32 {
            let value = rng.gen::<u32>();

            tree.insert(i, value);
        }

        tree.insert(1, 1);
        tree.insert(u32::MAX - 1, u32::MAX - 1);
        tree.insert(u32::MAX / 2, u32::MAX / 2);

        let mut tree: FractalTreeDisk<u32, u32> = tree.into();

        let dict = tree.create_zstd_dict();
        println!("Dict size: {}", dict.len());

        let mut tree_no_compression = tree.clone();
        let out_buf = Vec::new();
        let mut out_buf = std::io::Cursor::new(out_buf);
        tree_no_compression.write_to_buffer(&mut out_buf).unwrap();
        println!("Size without compression: {}", out_buf.into_inner().len());

        let mut tree_compression = tree.clone();
        tree_compression.set_compression(libcompression::CompressionConfig {
            compression_type: libcompression::CompressionType::ZSTD,
            // compression_dict: Some(std::sync::Arc::new(dict)),
            compression_dict: None,
            compression_level: -1,
        });
        let out_buf = Vec::new();
        let mut out_buf = std::io::Cursor::new(out_buf);
        tree_compression.write_to_buffer(&mut out_buf).unwrap();
        println!("Size with compression: {}", out_buf.into_inner().len());

        let mut tree_dict = tree.clone();
        tree_dict.set_compression(libcompression::CompressionConfig {
            compression_type: libcompression::CompressionType::ZSTD,
            compression_dict: Some(dict),
            // compression_dict: None,
            compression_level: -1,
        });
        let out_buf = Vec::new();
        let mut out_buf = std::io::Cursor::new(out_buf);
        tree_dict.write_to_buffer(&mut out_buf).unwrap();
        println!(
            "Size with compression + dict: {}",
            out_buf.into_inner().len()
        );
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
            bincode::decode_from_std_read(
                &mut buf,
                bincode::config::standard().with_fixed_int_encoding(),
            )
            .unwrap();
        assert!(tree.root.keys == orig_root_keys);

        let result = tree.search(&mut buf, &1);
        assert!(result.is_some());
        assert!(result.unwrap() == 1);

        let result = tree.search(&mut buf, &(u32::MAX - 1));
        assert!(result.is_some());
        assert!(result.unwrap() == u32::MAX - 1);

        let result = tree.search(&mut buf, &(u32::MAX / 2));
        assert!(result.is_some());
        assert!(result.unwrap() == u32::MAX / 2);

        let mut tree: FractalTreeBuild<u32, u64> =
            FractalTreeBuild::new(128, 256);
        for i in 0..1024 * 1024 {
            tree.insert(i, i as u64);
        }

        let mut tree: FractalTreeDisk<u32, u64> = tree.into();
        tree.set_compression(libcompression::CompressionConfig::default());
        let mut buffer =
            std::io::BufWriter::new(std::io::Cursor::new(Vec::new()));
        // Push some dummy bytes at the front
        buffer.write(&[0; 1024]).unwrap();

        let tree_loc = tree.write_to_buffer(&mut buffer).unwrap();
        let raw_vec = buffer.into_inner().unwrap().into_inner();
        let mut buf = std::io::BufReader::new(std::io::Cursor::new(raw_vec));

        let mut tree: FractalTreeDisk<u32, u64> =
            FractalTreeDisk::from_buffer(&mut buf, tree_loc).unwrap();

        let result = tree.search(&mut buf, &1);
        assert!(result.is_some());
    }

    #[ignore]
    #[test]
    /// This is mostly to see how large a very large tree is (in gigs,
    /// and megabytes)
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
        let size2: SpecificSize<human_size::multiples::Gigabyte> =
            format!("{} B", raw_vec.len()).parse().unwrap();
        println!("Raw Vec Size: {}", size2);
        let size2: SpecificSize<human_size::multiples::Megabyte> =
            format!("{} B", raw_vec.len()).parse().unwrap();
        println!("Raw Vec Size: {}", size2);

        let mut buf = std::io::BufReader::new(std::io::Cursor::new(raw_vec));

        buf.seek(SeekFrom::Start(tree_loc)).unwrap();
        let mut tree: FractalTreeDisk<u32, u32> =
            bincode::decode_from_std_read(
                &mut buf,
                bincode::config::standard().with_fixed_int_encoding(),
            )
            .unwrap();
        assert!(tree.root.keys == orig_root_keys);

        let result = tree.search(&mut buf, &1);
        assert!(result.is_some());
        assert!(result.unwrap() == 1);

        let result = tree.search(&mut buf, &(u32::MAX - 1));
        assert!(result.is_some());
        assert!(result.unwrap() == u32::MAX - 1);

        let result = tree.search(&mut buf, &(u32::MAX / 2));
        assert!(result.is_some());
        assert!(result.unwrap() == u32::MAX / 2);

        panic!();
    }
}
