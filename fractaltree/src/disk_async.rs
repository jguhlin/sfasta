// This whole file is behind the async flag, so we don't need to worry
// about

use std::{ops::SubAssign, sync::Arc};

use bincode::{BorrowDecode, Decode, Encode};
use futures::future::{BoxFuture, FutureExt};
use pulp::Arch;
use tokio::{
    fs::File,
    io::{
        AsyncBufRead, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt,
        BufReader, SeekFrom,
    },
    sync::{OwnedRwLockWriteGuard, OwnedRwLockReadGuard, RwLock},
};

use crate::*;
use libcompression::*;

/// This is the on-disk version of the FractalTree
///
/// The root node is loaded with the fractal tree, but children are
/// loaded on demand
#[derive(Debug, Clone)]
pub struct FractalTreeDiskAsync<K: Key, V: Value>
{
    pub root: Arc<RwLock<NodeDiskAsync<K, V>>>,
    pub start: u64, /* On disk position of the fractal tree, such that
                     * all locations are start + offset */
    pub compression: Option<CompressionConfig>,
    pub file_handles: Arc<RwLock<Vec<Arc<RwLock<BufReader<File>>>>>>,
    pub file: Option<String>,
}

impl<K: Key, V: Value> FractalTreeDiskAsync<K, V>
{
    pub async fn get_filehandle(&self)
        -> OwnedRwLockWriteGuard<BufReader<File>>
    {
        let file_handles = Arc::clone(&self.file_handles);

        loop {
            let file_handles_read = file_handles.read().await;

            // Loop through each until we find one that is not locked
            for file_handle in file_handles_read.iter() {
                let file_handle = Arc::clone(file_handle);
                let file_handle = file_handle.try_write_owned();
                if let Ok(file_handle) = file_handle {
                    return file_handle;
                }
            }

            if file_handles_read.len() < 128 {
                // There are no available file handles, so we need to create a
                // new one and the number isn't crazy (yet)
                break;
            }

            drop(file_handles_read);

            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }

        // Otherwise, create one and add it to the list
        let file = tokio::fs::File::open(self.file.as_ref().unwrap())
            .await
            .unwrap();
        let file_handle = Arc::new(RwLock::new(BufReader::new(file)));

        let mut write_lock = file_handles.write().await;

        write_lock.push(Arc::clone(&file_handle));

        file_handle.try_write_owned().unwrap()
    }
}

impl<K: Key, V: Value> Decode for FractalTreeDiskAsync<K, V>
{
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let compression: Option<CompressionConfig> =
            bincode::Decode::decode(decoder)?;
        let start: u64 = bincode::Decode::decode(decoder)?;
        let mut root: NodeDiskAsync<K, V> = if compression.is_some() {
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

        root.is_root = true;

        Ok(FractalTreeDiskAsync {
            root: Arc::new(RwLock::new(root)),
            start,
            compression,
            file_handles: Arc::new(RwLock::new(Vec::new())),
            file: None,
        })
    }
}

impl<K: Key, V: Value> Default for FractalTreeDiskAsync<K, V>
{
    fn default() -> Self
    {
        FractalTreeDiskAsync {
            root: Arc::new(RwLock::new(NodeDiskAsync {
                is_root: true,
                state: NodeStateAsync::InMemory,
                is_leaf: true,
                keys: Vec::new(),
                children: None,
                values: None,
                children_in_memory: false,
            })),
            start: 0,
            compression: None, // Default to no compression
            file_handles: Arc::new(RwLock::new(Vec::new())),
            file: None,
        }
    }
}

impl<K: Key, V: Value> FractalTreeDiskAsync<K, V>
{
    pub async fn load_tree(
        &self,
        in_buf: &mut OwnedRwLockWriteGuard<BufReader<File>>,
    ) -> Result<(), &'static str>
    {
        let mut root = self.root.write().await;
        root.load_all(in_buf, &self.compression, self.start).await;
        Ok(())
    }

    pub async fn len(&self) -> Result<usize, &'static str>
    {
        let root = self.root.read().await;
        root.len().await
    }

    pub fn set_compression(&mut self, compression: CompressionConfig)
    {
        self.compression = Some(compression);
    }

    pub async fn search(
        &self,
        in_buf: &mut OwnedRwLockWriteGuard<BufReader<File>>,
        key: &K,
    ) -> Option<V>
    {       

        if self.root.read().await.children_in_memory {
            let root = self.root.read().await;
            return root.search_read(in_buf, &self.compression, self.start, key).await;
        } else {
            let mut root = self.root.write().await;

            if root.children_loaded().await {
                root.children_in_memory = true;
            }

            root.search(in_buf, &self.compression, self.start, key)
                .await
        }
    }

    pub async fn from_buffer(
        in_buf: &mut BufReader<File>,
        pos: u64,
    ) -> Result<Self, &'static str>
    {
        in_buf.seek(SeekFrom::Start(pos)).await.unwrap();
        let bincode_config =
            bincode::config::standard().with_variable_int_encoding();

        let tree: FractalTreeDiskAsync<K, V> =
            match bincode_decode_from_buffer_async_with_size_hint::<
                { 64 * 1024 },
                _,
                _,
            >(in_buf, bincode_config)
            .await
            {
                Ok(x) => x,
                Err(_) => {
                    return Result::Err("Failed to decode FractalTreeDisk")
                }
            };

        Ok(tree)
    }

    pub async fn from_buffer_async(
        in_buf: &mut tokio::io::BufReader<tokio::fs::File>,
        pos: u64,
    ) -> Result<Self, &'static str>
    {
        in_buf.seek(SeekFrom::Start(pos)).await.unwrap();
        let bincode_config =
            bincode::config::standard().with_variable_int_encoding();

        let tree: FractalTreeDiskAsync<K, V> =
            match bincode_decode_from_buffer_async_with_size_hint::<
                { 4 * 1024 },
                _,
                _,
            >(in_buf, bincode_config)
            .await
            {
                Ok(x) => x,
                Err(_) => {
                    return Result::Err("Failed to decode FractalTreeDisk")
                }
            };
        Ok(tree)
    }
}

#[derive(Debug, Clone)]
pub enum NodeStateAsync
{
    InMemory,
    Compressed(Vec<u8>),
    OnDisk(u32),
}

impl NodeStateAsync
{
    pub fn as_ref(&self) -> Option<u32>
    {
        match self {
            NodeStateAsync::OnDisk(x) => Some(*x),
            _ => None,
        }
    }

    pub fn on_disk(&self) -> bool
    {
        match self {
            NodeStateAsync::OnDisk(_) => true,
            _ => false,
        }
    }

    pub fn compressed(&self) -> bool
    {
        match self {
            NodeStateAsync::Compressed(_) => true,
            _ => false,
        }
    }

    pub fn in_memory(&self) -> bool
    {
        match self {
            NodeStateAsync::InMemory => true,
            _ => false,
        }
    }

    pub fn loc(&self) -> u32
    {
        match self {
            NodeStateAsync::OnDisk(x) => *x,
            _ => panic!("Node not on disk"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeDiskAsync<K, V>
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub state: NodeStateAsync,
    pub keys: Vec<K>,
    pub children: Option<Arc<RwLock<Vec<Arc<RwLock<NodeDiskAsync<K, V>>>>>>>,
    pub values: Option<Vec<V>>,
    pub children_in_memory: bool,
}

impl<K: Key, V: Value> Decode for NodeDiskAsync<K, V>
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
            let children: Vec<Arc<RwLock<NodeDiskAsync<K, V>>>> = locs
                .iter()
                .map(|x| {
                    Arc::new(RwLock::new(NodeDiskAsync::<K, V>::from_loc(*x)))
                })
                .collect::<Vec<_>>();
            Some(Arc::new(RwLock::new(children)))
        };

        Ok(NodeDiskAsync {
            is_root: false,
            is_leaf,
            state: NodeStateAsync::InMemory,
            keys,
            children,
            values,
            children_in_memory: false,
        })
    }
}

impl<K: Key, V: Value> BorrowDecode<'_> for NodeDiskAsync<K, V>
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
            let children: Vec<Arc<RwLock<NodeDiskAsync<K, V>>>> = locs
                .iter()
                .map(|x| {
                    Arc::new(RwLock::new(NodeDiskAsync::<K, V>::from_loc(*x)))
                })
                .collect::<Vec<_>>();
            Some(Arc::new(RwLock::new(children)))
        };

        Ok(NodeDiskAsync {
            is_root: false,
            is_leaf,
            state: NodeStateAsync::InMemory,
            keys: keys.to_vec(),
            children,
            values,
            children_in_memory: false,
        })
    }
}

impl<K: Key, V: Value> NodeDiskAsync<K, V>
{
    pub fn from_loc(loc: u32) -> Self
    {
        NodeDiskAsync {
            is_root: false,
            is_leaf: false,
            state: NodeStateAsync::OnDisk(loc),
            keys: Vec::new(),
            children: None,
            values: None,
            children_in_memory: false,
        }
    }

    // todo move load logic to FractalTreeDisk so we don't have to pass a
    // file handle around
    pub async fn load(
        &mut self,
        in_buf: &mut OwnedRwLockWriteGuard<BufReader<File>>,
        compression: &Option<CompressionConfig>,
        start: u64,
    )
    {
        in_buf
            .seek(SeekFrom::Start(start + self.state.loc() as u64))
            .await
            .unwrap();

        let config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<{ 8 * 1024 * 1024 }>();

        *self = if compression.is_some() {
            let compressed: Vec<u8> =
                bincode_decode_from_buffer_async_with_size_hint::<
                    { 64 * 1024 },
                    _,
                    _,
                >(in_buf, config)
                .await
                .unwrap();
            let decompressed = compression
                .as_ref()
                .unwrap()
                .decompress_async(&compressed)
                .await.unwrap();
            bincode::decode_from_slice(&decompressed, config).unwrap().0
        } else {
            bincode_decode_from_buffer_async_with_size_hint::<{64 * 1024}, _, _>(in_buf, config).await.unwrap()
        };

        delta_decode(&mut self.keys);
    }

    // todo: This doesn't work, need to account for compression better
    pub async fn load_all(
        &mut self,
        in_buf: &mut OwnedRwLockWriteGuard<BufReader<File>>,
        compression: &Option<CompressionConfig>,
        start: u64,
    )
    {
        if self.state.on_disk() {
            self.load(in_buf, compression, start).await;
        }

        if !self.is_leaf {
            let mut children = self.children.as_ref().unwrap().write().await;

            for child in children.iter_mut() {
                let mut child = child.write().await;
                Box::pin(child.load_all(in_buf, &compression, start)).await;
            }
        }

        self.state = NodeStateAsync::InMemory;
    }

    pub async fn search(
        &mut self,
        in_buf: &mut OwnedRwLockWriteGuard<BufReader<File>>,
        compression: &Option<CompressionConfig>,
        start: u64,
        key: &K,
    ) -> Option<V>
    {
        if self.state.on_disk() {
            self.load(in_buf, compression, start).await;
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

            let children = self.children.as_ref().expect("Not a leaf but children are empty").read().await;

            if children[i].read().await.children_in_memory {
                let child = children[i].read().await;
                return Box::pin(child.search_read(in_buf, compression, start, key)).await
            } else {
                let mut child = children[i].write().await;
                child.children_in_memory = child.children_loaded().await;

                Box::pin(child.search(in_buf, compression, start, key)).await
            }
        }
    }

    pub async fn search_read(
        &self,
        in_buf: &mut OwnedRwLockWriteGuard<BufReader<File>>,
        compression: &Option<CompressionConfig>,
        start: u64,
        key: &K,
    ) -> Option<V>
    {
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

            let children = self.children.as_ref().expect("Not a leaf but children are empty").read().await;

            if children[i].read().await.children_in_memory {
                let child = children[i].read().await;
                return Box::pin(child.search_read(in_buf, compression, start, key)).await
            } else {
                let mut child = children[i].write().await;
                child.children_in_memory = child.children_loaded().await;

                Box::pin(child.search(in_buf, compression, start, key)).await
            }
        }
    }

    pub async fn children_stored_on_disk(&self) -> bool
    {
        if self.is_leaf {
            true
        } else {
            let children = self.children.as_ref().unwrap().read().await;
            for child in children.iter() {
                if !child.read().await.state.on_disk() {
                    return false;
                }
            }
            true
        }
    }

    pub async fn children_loaded(&self) -> bool
    {
        if self.is_leaf {
            true
        } else if self.state.on_disk() {
            false
        } else {
            let children = self.children.as_ref().unwrap().read().await;
            for child in children.iter() {
                if child.read().await.state.on_disk() {
                    return false;
                }
            }
            true
        }        
    }

    pub async fn len(&self) -> Result<usize, &'static str>
    {
        if self.is_leaf {
            Ok(self.keys.len())
        } else {
            let mut len = 0;
            if self.children.is_none() {
                return Err("Children not loaded");
            }

            let children = self.children.as_ref().unwrap().read().await;

            for child in children.iter() {
                len += Box::pin(child.read().await.len()).await?;
            }

            Ok(len)
        }
    }
}

// todo curry size_hint as const
pub(crate) async fn bincode_decode_from_buffer_async_with_size_hint<
    const SIZE_HINT: usize,
    T,
    C,
>(
    in_buf: &mut tokio::io::BufReader<tokio::fs::File>,
    bincode_config: C,
) -> Result<T, String>
where
    T: bincode::Decode,
    C: bincode::config::Config,
{
    let mut buf = vec![0; SIZE_HINT];
    match in_buf.read(&mut buf).await {
        Ok(_) => (),
        Err(_) => return Result::Err("Failed to read buffer".to_string()),
    }

    loop {
        match bincode::decode_from_slice(&buf, bincode_config) {
            Ok(x) => {
                return Ok(x.0);
            }
            Err(_) => {
                let orig_length = buf.len();
                let doubled = buf.len() * 2;

                buf.resize(doubled, 0);

                match in_buf.read(&mut buf[orig_length..]).await {
                    Ok(_) => (),
                    Err(_) => {
                        return Result::Err("Failed to read buffer".to_string())
                    }
                }

                if doubled > 16 * 1024 * 1024 {
                    return Result::Err("Failed to decode bincode".to_string());
                }
            }
        }
    }
}
