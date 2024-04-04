use super::*;
use std::io::{Read, Seek, SeekFrom, Write};

use bincode::{BorrowDecode, Decode, Encode};
use binout::{Serializer, VByte};
use libcompression::*;
use pulp::Arch;

// Tried to make it generic, didn't work
// Maybe try again later...

/// This is the on-disk version of the FractalTree
///
/// The root node is loaded with the fractal tree, but children are loaded on demand
#[derive(Encode, Decode, Debug)]
pub struct FractalTreeDisk
{
    pub root: NodeDisk,
    pub start: u64, /* On disk position of the fractal tree, such that
                     * all locations are start + offset */
}

impl Default for FractalTreeDisk
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
        }
    }
}

impl FractalTreeDisk
{
    pub fn search<R>(&mut self, in_buf: &mut R, key: u64) -> Option<&u32>
    where
        R: Read + Seek + Send + Sync,
    {
        self.root.is_root = true; // TODO: Move this to custom decode
        self.root.search(in_buf, self.start, key)
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: W) -> Result<u64, &str>
    where
        W: Write + Seek,
    {
        let start_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();

        self.root.store(&mut out_buf, start_pos);

        self.start = start_pos;

        let tree_location = out_buf.seek(SeekFrom::Current(0)).unwrap();

        let config = bincode::config::standard().with_variable_int_encoding();
        bincode::encode_into_std_write(&*self, &mut out_buf, config).unwrap();

        Ok(tree_location)
    }
}

#[derive(Debug)]
pub struct NodeDisk
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub state: Option<u32>, // None means in memory, Some(u32) is the location on the disk
    pub keys: Vec<u64>,
    pub children: Option<Vec<Box<NodeDisk>>>,
    pub values: Option<Vec<u32>>,
}

// Concrete types, u64 u32
impl Encode for NodeDisk
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

impl Decode for NodeDisk
{
    fn decode<D: bincode::de::Decoder>(decoder: &mut D) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let is_leaf: bool = bincode::Decode::decode(decoder)?;
        let keys: Vec<u64> = bincode::Decode::decode(decoder)?;

        let values: Option<Vec<u32>> = if is_leaf {
            bincode::Decode::decode(decoder)?
        } else {
            None
        };

        let children = if is_leaf {
            None
        } else {
            let locs: Vec<u32> = bincode::Decode::decode(decoder)?;
            let children: Vec<Box<NodeDisk>> = locs
                .iter()
                .map(|x| Box::new(NodeDisk::from_loc(*x)))
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

impl BorrowDecode<'_> for NodeDisk
{
    fn borrow_decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let is_leaf: bool = bincode::Decode::decode(decoder)?;

        let keys: Vec<u8> = bincode::Decode::decode(decoder)?;
        let mut keys: Box<[u64]> = VByte::read_array(&mut &keys[..]).unwrap();
        let delta = keys[0];
        let arch = Arch::new();
        arch.dispatch(|| {
            keys[1..].iter_mut().for_each(|x| *x += delta);
        });

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
            let children: Vec<Box<NodeDisk>> = locs
                .iter()
                .map(|x| Box::new(NodeDisk::from_loc(*x)))
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

impl NodeDisk
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

    pub fn load<R>(&mut self, in_buf: &mut R)
    where
        R: Read + Seek,
    {
        let config = bincode::config::standard().with_variable_int_encoding();

        in_buf.seek(SeekFrom::Start(self.state.unwrap() as u64)).unwrap();
        let node: NodeDisk = bincode::decode_from_std_read(in_buf, config).unwrap();
        *self = node;
    }

    pub fn store<W>(&mut self, out_buf: &mut W, start: u64)
    where
        W: Write + Seek,
    {
        let config = bincode::config::standard().with_variable_int_encoding();

        // Make sure all the children are stored first...
        if !self.is_leaf {
            for child in self.children.as_mut().unwrap() {
                if !child.state.is_some() {
                    child.store(out_buf, start);
                }
            }
        }

        // Don't store the root separately...
        if !self.is_root {
            let cur_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
            bincode::encode_into_std_write(&*self, out_buf, config).unwrap();

            self.state = Some(cur_pos as u32 - start as u32);
            self.children = None;
            self.values = None;
        }
    }

    pub fn search<R>(&mut self, in_buf: &mut R, start: u64, key: u64) -> Option<&u32>
    where
        R: Read + Seek + Send + Sync,
    {

        if self.state.is_some() {
            self.load(in_buf);
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

            self.children.as_mut().unwrap()[i].search(in_buf, start, key)
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
}

#[cfg(test)]
mod tests
{
    use super::*;
    use human_size::SpecificSize;
    use rand::prelude::*;
    use xxhash_rust::xxh3::xxh3_64;

    #[test]
    fn node_storage()
    {
        let rng = thread_rng();
        // Get 128 random values
        let values: Vec<u32> = rng.sample_iter(rand::distributions::Standard).take(128).collect();

        let rng = thread_rng();
        let mut keys: Vec<u64> = rng.sample_iter(rand::distributions::Standard).take(128).collect();
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
        node.store(&mut buf, 0);

        let len = buf.into_inner().unwrap().into_inner().len();
        let size: SpecificSize<human_size::Kilobyte> = format!("{} B", len).parse().unwrap();

        println!("Node size: {}", size);

        let size: SpecificSize<human_size::Kilobyte> = format!("{} B", no_vbyte_len).parse().unwrap();
        println!("Node size (no vbyte): {}", size);
    }

    #[test]
    fn tree_storage() {
        let mut rng = thread_rng();

        let mut tree = FractalTreeBuild::new(128, 256);

        // Generate 1024 * 1024 random key value pairs

        for _ in 0..1024 * 1024 {
            let key = rng.gen::<u64>();
            let value = rng.gen::<u32>();

            tree.insert(key, value);
        }

        // Guaranteed insert to try and find later
        tree.insert(1, 1);
        tree.insert(u64::MAX - 1, u32::MAX - 1);
        tree.insert(u64::MAX / 2, u32::MAX / 2);

        let mut tree: FractalTreeDisk = tree.into();
        let orig_root_keys = tree.root.keys.clone();

        let mut buf = std::io::BufWriter::new(std::io::Cursor::new(Vec::new()));
        let tree_loc = tree.write_to_buffer(&mut buf).unwrap();

        // --------------------------------------------
        // Load up the tree now

        let raw_vec = buf.into_inner().unwrap().into_inner();

        println!("Raw Vec Len: {}", raw_vec.len());

        let mut buf = std::io::BufReader::new(std::io::Cursor::new(raw_vec));

        buf.seek(SeekFrom::Start(tree_loc)).unwrap();
        let mut tree: FractalTreeDisk = bincode::decode_from_std_read(&mut buf, bincode::config::standard().with_variable_int_encoding()).unwrap();
        assert!(tree.root.keys == orig_root_keys);

        let result = tree.search(&mut buf, 1);
        assert!(result.is_some());
        assert!(*result.unwrap() == 1);

        let result = tree.search(&mut buf, u64::MAX - 1);
        assert!(result.is_some());
        assert!(*result.unwrap() == u32::MAX - 1);

        let result = tree.search(&mut buf, u64::MAX / 2);
        assert!(result.is_some());
        assert!(*result.unwrap() == u32::MAX / 2);
    }
}
