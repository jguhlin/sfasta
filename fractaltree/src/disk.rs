use super::*;
use std::io::{Read, Seek, SeekFrom, Write};

use binout::{Serializer, VByte};
use libcompression::*;
// use pulp::Arch;

/// This is the on-disk version of the FractalTree
pub struct FractalTreeDisk<K, V>
where
    K: Key,
    V: Value,
{
    pub root: NodeDiskState<K, V>,
    pub start: u64, // On disk position of the fractal tree, such that
    // all locations are start + offset
    pub compression_config: CompressionConfig, // TODO
}

impl<K, V> Default for FractalTreeDisk<K, V>
where
    K: Key,
    V: Value,
{
    fn default() -> Self {
        FractalTreeDisk {
            root: NodeDiskState::InMemory(Box::new(NodeDisk {
                is_root: true,
                is_leaf: true,
                keys: Vec::new(),
                children: None,
                values: None,
            })),
            start: 0,

            // This is the default for the FractalTree, but TODO benchmarks
            compression_config: CompressionConfig {
                compression_type: CompressionType::ZSTD,
                compression_level: -9,
                compression_dict: None,
            },
        }
    }
}

pub fn load_on_disk_node<R>(in_buf: &mut R, start: u64, loc: u32) -> NodeDisk<u64, u32>
where
    R: Read + Seek,
{
    let config = bincode::config::standard().with_fixed_int_encoding();

    in_buf
        .seek(SeekFrom::Start(start as u64 + loc as u64))
        .unwrap();
    let node: NodeDisk<u64, u32> = bincode::decode_from_std_read(in_buf, config).unwrap();
    node
}

pub fn store_node<R, K, V>(mut out_buf: &mut R, start: u64, node: NodeDisk<K, V>) -> NodeDiskState<u64, u32>
where
    NodeDisk<K, V>: Encode,
    K: Key,
    V: Value,
    R: Write + Seek
{
    let config = bincode::config::standard().with_fixed_int_encoding();

    let cur_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
    bincode::encode_into_std_write(&node, &mut out_buf, config).unwrap();

    NodeDiskState::from_loc((start as u64 - cur_pos as u64) as u32)
}

impl<'fractal> FractalTreeDisk<u64, u32> {
    pub fn search<R>(&mut self, mut in_buf: R, key: u64) -> Option<&u32>
    where
        R: 'fractal + Read + Seek + Send + Sync,
    {
        if !self.root.is_loaded() {
            let node = load_on_disk_node(&mut in_buf, self.start, self.root.loc());
            self.root = NodeDiskState::InMemory(Box::new(node));
        }

        self.root.search(&mut in_buf, self.start, key)
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: W) -> Result<u64, &str>
    where 
        W: 'fractal + Write + Seek
    {
        let start_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();

        

        Ok(start_pos)
    }
}

#[derive(Debug)]
pub enum NodeDiskState<K, V>
where
    K: Key,
    V: Value,
{
    InMemory(Box<NodeDisk<K, V>>),
    OnDisk(u32),
}

impl NodeDiskState<u64, u32> {
    pub fn is_loaded(&self) -> bool {
        match self {
            NodeDiskState::InMemory(_) => true,
            NodeDiskState::OnDisk(_) => false,
        }
    }

    pub fn loc(&self) -> u32 {
        match self {
            NodeDiskState::InMemory(_) => panic!("Node is in memory"),
            NodeDiskState::OnDisk(loc) => *loc,
        }
    }

    pub fn from_loc(loc: u32) -> Self {
        NodeDiskState::OnDisk(loc)
    }

    pub fn search<R>(&mut self, in_buf: &mut R, start: u64, key: u64) -> Option<&u32>
    where
        R: Read + Seek + Send + Sync,
    {
        match self {
            NodeDiskState::InMemory(node) => node.search(in_buf, start, key),
            NodeDiskState::OnDisk(loc) => {
                let node = load_on_disk_node(in_buf, start, *loc);
                *self = NodeDiskState::InMemory(Box::new(node));
                self.search(in_buf, start, key)
            }
        }
    }
}

#[derive(Debug)]
pub struct NodeDisk<K, V>
where
    K: Key,
    V: Value,
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub keys: Vec<K>,
    pub children: Option<Vec<Box<NodeDiskState<K, V>>>>,
    pub values: Option<Vec<V>>,
}

// Concrete types, u64 u32
impl Encode for NodeDisk<u64, u32> {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> core::result::Result<(), bincode::error::EncodeError> {

        bincode::Encode::encode(&self.is_leaf, encoder)?;

        let start_key = self.keys[0];
        let mut keys = vec![start_key];
        keys.extend(self.keys[1..].iter().map(|x| x - start_key).collect::<Vec<_>>());

        let mut output: Vec<u8> = Vec::with_capacity(VByte::array_size(&keys[..]));
        VByte::write_array(&mut output, &keys[..]).unwrap();

        bincode::Encode::encode(&output, encoder)?;

        if self.is_leaf {
            let mut output: Vec<u8> =
                Vec::with_capacity(VByte::array_size(&self.values.as_ref().unwrap()[..]));
            VByte::write_array(&mut output, &self.values.as_ref().unwrap()[..]).unwrap();
            bincode::Encode::encode(&output, encoder)?;
        } else {
            let locs = self
                .children
                .as_ref()
                .unwrap()
                .iter()
                .map(|x| x.loc())
                .collect::<Vec<u32>>();

            let mut output: Vec<u8> = Vec::with_capacity(VByte::array_size(&locs[..]));
            VByte::write_array(&mut output, &locs[..]).unwrap();
            bincode::Encode::encode(&output, encoder)?;
        }
        Ok(())
    }
}

impl Decode for NodeDisk<u64, u32> {
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, bincode::error::DecodeError> {
        let keys: Vec<u8> = bincode::Decode::decode(decoder)?;
        let keys: Box<[u64]> = VByte::read_array(&mut &keys[..]).unwrap();
        let is_leaf = match keys[0] {
            0 => false,
            1 => true,
            _ => panic!("Invalid is_leaf value"),
        };

        let values = if is_leaf {
            let values: Vec<u8> = bincode::Decode::decode(decoder)?;
            let values: Box<[u32]> = VByte::read_array(&mut &values[..]).unwrap();
            Some(values.to_vec())
        } else {
            None
        };

        let children = if is_leaf {
            None
        } else {
            let locs: Vec<u8> = bincode::Decode::decode(decoder)?;
            let locs: Box<[u32]> = VByte::read_array(&mut &locs[..]).unwrap();
            let children: Vec<Box<NodeDiskState<u64, u32>>> = locs
                .iter()
                .map(|x| Box::new(NodeDiskState::from_loc(*x)))
                .collect::<Vec<_>>();
            Some(children)
        };

        Ok(NodeDisk {
            is_root: false,
            is_leaf,
            keys: keys[1..].to_vec(),
            children,
            values,
        })
    }
}

impl NodeDisk<u64, u32> {
    pub fn search<R>(&mut self, mut in_buf: R, start: u64, key: u64) -> Option<&u32>
    where
        R: Read + Seek + Send + Sync,
    {
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

            self.children.as_mut().unwrap()[i].search(&mut in_buf, start, key)
        }
    }

    pub fn all_stored_on_disk(&self) -> bool {
        if self.is_leaf {
            true
        } else {
            self.children.as_ref().unwrap().iter().all(|x| !x.is_loaded())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use human_size::SpecificSize;
    use xxhash_rust::xxh3::xxh3_64;
    use rand::prelude::*;

    #[test]
    fn node_storage() {

        let rng = thread_rng();
        // Get 128 random values
        let values: Vec<u32> = rng.sample_iter(rand::distributions::Standard).take(128).collect();

        let rng = thread_rng();
        let mut keys: Vec<u64> = rng.sample_iter(rand::distributions::Standard).take(128).collect();
        keys.sort();

        let node = NodeDisk {
            is_root: true,
            is_leaf: true,
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
        store_node(&mut buf, 0, node);

        let len = buf.into_inner().unwrap().into_inner().len();
        let size: SpecificSize<human_size::Kilobyte> =
            format!("{} B", len).parse().unwrap();

        println!("Node size: {}", size);

        let size: SpecificSize<human_size::Kilobyte> =
            format!("{} B", no_vbyte_len).parse().unwrap();
        println!("Node size (no vbyte): {}", size);
        panic!();

        
    }

}