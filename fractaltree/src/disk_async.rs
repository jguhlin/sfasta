// This whole file is behind the async flag, so we don't need to worry
// about

use std::{
    async_iter::AsyncIterator,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    collections::BTreeMap,
};

use bincode::{BorrowDecode, Decode};
use tokio::{
    fs::File,
    io::{AsyncReadExt, BufReader, AsyncSeekExt, SeekFrom},
    sync::{
        Mutex, OwnedMutexGuard, RwLock,
    },
};

#[cfg(unix)]
use std::os::fd::AsRawFd;

use crate::*;
use libcompression::*;

/// This is the on-disk version of the FractalTree
///
/// The root node is loaded with the fractal tree, but children are
/// loaded on demand
#[derive(Debug)]
pub struct FractalTreeDiskAsync<K: Key, V: Value>
{
    pub root: Arc<RwLock<NodeDiskAsync<K, V>>>,
    pub start: u64, /* On disk position of the fractal tree, such that
                     * all locations are start + offset */
    pub compression: Arc<Option<CompressionConfig>>,
    pub file_handles: Arc<RwLock<Vec<Arc<RwLock<BufReader<File>>>>>>,
    pub file: Option<String>,
    pub file_handle_manager: Arc<AsyncFileHandleManager>,

    // Which nodes have been opened, in case they haven't been placed on the tree yet
    // such as from a DFS search and "right" thing
    pub opened: Arc<RwLock<BTreeMap<u64, Arc<RwLock<NodeDiskAsync<K, V>>>>>>,
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

        let compression = Arc::new(compression);

        Ok(FractalTreeDiskAsync {
            root: Arc::new(RwLock::with_max_readers(root, 128)),
            start,
            compression,
            file_handles: Arc::new(RwLock::with_max_readers(Vec::new(), 16)),
            file: None,
            file_handle_manager: Default::default(),
            opened: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }
}

impl<K: Key, V: Value> Default for FractalTreeDiskAsync<K, V>
{
    fn default() -> Self
    {
        FractalTreeDiskAsync {
            root: Arc::new(RwLock::with_max_readers(
                NodeDiskAsync {
                    is_root: true,
                    state: NodeStateAsync::InMemory,
                    is_leaf: true,
                    keys: Vec::new(),
                    children: None,
                    values: None,
                    children_in_memory: false,
                    left: None,
                    right: None,
                    loc: 0,
                },
                128,
            )),
            start: 0,
            compression: Arc::new(None), // Default to no compression
            file_handles: Arc::new(RwLock::with_max_readers(Vec::new(), 8)),
            file: None,
            file_handle_manager: Default::default(),
            opened: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }
}

impl<K: Key, V: Value> FractalTreeDiskAsync<K, V>
{
    
    pub async fn load_tree(&self) -> Result<(), &'static str>
    {
        let root = Arc::clone(&self.root);
        let mut root = root.write_owned().await;
        root.load_all(
            Arc::clone(&self.file_handle_manager),
            Arc::clone(&self.compression),
            self.start,
        )
        .await;
        Ok(())
    }

    
    pub async fn len(&self) -> Result<usize, &'static str>
    {
        let root = self.root.read().await;
        root.len().await
    }

    pub fn set_compression(&mut self, compression: CompressionConfig)
    {
        self.compression = Arc::new(Some(compression));
    }

    
    pub async fn search(&self, key: &K) -> Option<V>
    {
        if self.root.read().await.children_in_memory {
            let root = self.root.read().await;
            return root
                .search_read(
                    Arc::clone(&self.file_handle_manager),
                    Arc::clone(&self.compression),
                    self.start,
                    key,
                )
                .await;
        } else {
            let mut root = self.root.write().await;

            if root.children_loaded().await {
                root.children_in_memory = true;
            }

            root.search(
                Arc::clone(&self.file_handle_manager),
                Arc::clone(&self.compression),
                self.start,
                key,
            )
            .await
        }
    }

    /*
    /// Find the first leaf of the node
    pub async fn first_leaf(
        &self,
    ) -> Arc<RwLock<NodeDiskAsync<K, V>>>
    {
        let root = self.root.read().await;
        let mut current_leaf = root;
        while !current_leaf.is_leaf {
            // Load node if it's not in memory
            if current_leaf.state.on_disk() {
                let mut in_buf = self.file_handle_manager.get_filehandle().await;
                current_leaf.load(&mut in_buf, &self.compression, self.start).await;
            }
            let children = current_leaf.children.as_ref().unwrap().read().await;
            current_leaf = children[0].read().await;
        }

        current_leaf
    }
    */

    
    pub async fn from_buffer(
        file: String,
        pos: u64,
    ) -> Result<Self, &'static str>
    {
        let fhm = AsyncFileHandleManager {
            file_handles: Arc::new(RwLock::with_max_readers(Vec::new(), 128)),
            file_name: Some(file.clone()),
        };

        let mut in_buf = fhm.get_filehandle().await;

        in_buf.seek(SeekFrom::Start(pos)).await.unwrap();
        let bincode_config =
            bincode::config::standard().with_variable_int_encoding();

        let mut tree: FractalTreeDiskAsync<K, V> =
            match bincode_decode_from_buffer_async_with_size_hint::<
                { 64 * 1024 },
                _,
                _,
            >(&mut in_buf, bincode_config)
            .await
            {
                Ok(x) => x,
                Err(_) => {
                    return Result::Err("Failed to decode FractalTreeDisk")
                }
            };

        tree.file = Some(file);
        tree.file_handle_manager = Arc::new(fhm);

        Ok(tree)
    }
    
    pub async fn load_node(
        &self,
        fhm: Arc<AsyncFileHandleManager>,
        compression: &Arc<Option<CompressionConfig>>,
        loc: u64,
        node: Arc<RwLock<NodeDiskAsync<K, V>>>,
    )
    {
        let mut node_write = node.write().await;
        let mut in_buf = fhm.get_filehandle().await;

        let config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<{ 8 * 1024 * 1024 }>();

        let mut loaded_node: NodeDiskAsync<K, V> = if compression.is_some() {
            let compressed: Vec<u8> =
                bincode_decode_from_buffer_async_with_size_hint::<
                    { 64 * 1024 },
                    _,
                    _,
                >(&mut in_buf, config)
                .await
                .unwrap();

            let compression = Arc::clone(&compression);
            let decompressed = tokio::task::spawn_blocking(move || {
                compression
                    .as_ref()
                    .as_ref()
                    .unwrap()
                    .decompress(&compressed)
                    .unwrap()
            });

            let decompressed = decompressed.await.unwrap();

            bincode::decode_from_slice(&decompressed, config).unwrap().0
        } else {
            bincode_decode_from_buffer_async_with_size_hint::<{32 * 1024}, _, _>(&mut in_buf, config).await.unwrap()
        };

        delta_decode(&mut loaded_node.keys);
        loaded_node.loc = loc;
        *node_write = loaded_node;

        // Release the lock
        drop(node_write);

        let mut opened = self.opened.write().await;
        opened.insert(loc, Arc::clone(&node));
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


    // These are not stored, but are populated dynamically after the tree is loaded
    pub left: Option<Arc<RwLock<NodeDiskAsync<K, V>>>>,
    pub right: Option<Arc<RwLock<NodeDiskAsync<K, V>>>>,
    pub loc: u64,
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
                    Arc::new(RwLock::with_max_readers(
                        NodeDiskAsync::<K, V>::from_loc(*x),
                        128,
                    ))
                })
                .collect::<Vec<_>>();
            Some(Arc::new(RwLock::with_max_readers(children, 128)))
        };

        Ok(NodeDiskAsync {
            is_root: false,
            is_leaf,
            state: NodeStateAsync::InMemory,
            keys,
            children,
            values,
            children_in_memory: false,
            left: None,
            right: None,
            loc: 0,
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
                    Arc::new(RwLock::with_max_readers(
                        NodeDiskAsync::<K, V>::from_loc(*x),
                        128,
                    ))
                })
                .collect::<Vec<_>>();
            Some(Arc::new(RwLock::with_max_readers(children, 128)))
        };

        Ok(NodeDiskAsync {
            is_root: false,
            is_leaf,
            state: NodeStateAsync::InMemory,
            keys: keys.to_vec(),
            children,
            values,
            children_in_memory: false,
            left: None,
            right: None,
            loc: 0,
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
            left: None,
            right: None,
            loc: 0,
        }
    }

    
    pub async fn load(
        &mut self,
        in_buf: &mut OwnedMutexGuard<BufReader<File>>,
        compression: &Arc<Option<CompressionConfig>>,
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

            let compression = Arc::clone(&compression);
            let decompressed = tokio::task::spawn_blocking(move || {
                compression
                    .as_ref()
                    .as_ref()
                    .unwrap()
                    .decompress(&compressed)
                    .unwrap()
            });

            let decompressed = decompressed.await.unwrap();

            bincode::decode_from_slice(&decompressed, config).unwrap().0
        } else {
            bincode_decode_from_buffer_async_with_size_hint::<{64 * 1024}, _, _>(in_buf, config).await.unwrap()
        };

        delta_decode(&mut self.keys);
    }

    // todo: This doesn't work, need to account for compression better
    
    pub fn load_all<'a>(
        &'a mut self,
        fhm: Arc<AsyncFileHandleManager>,
        compression: Arc<Option<CompressionConfig>>,
        start: u64,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>
    {
        Box::pin(async move {
            if self.state.on_disk() {
                let mut in_buf = fhm.get_filehandle().await;
                self.load(&mut in_buf, &compression, start).await;
            }

            if !self.is_leaf {
                let mut children =
                    self.children.as_ref().unwrap().write().await;

                let mut handles = Vec::new();

                for child in children.iter_mut() {
                    let child = Arc::clone(&child);
                    let fhm = Arc::clone(&fhm);
                    let compression = Arc::clone(&compression);
                    handles.push(tokio::spawn(async move {
                        let mut child = child.write_owned().await;
                        child.load_all(fhm, compression, start).await;
                    }));
                }

                for handle in handles {
                    handle.await.unwrap();
                }
            }

            self.state = NodeStateAsync::InMemory;
        })
    }

    
    pub async fn search(
        &mut self,
        fhm: Arc<AsyncFileHandleManager>,
        compression: Arc<Option<CompressionConfig>>,
        start: u64,
        key: &K,
    ) -> Option<V>
    {
        if self.state.on_disk() {
            let compression = Arc::clone(&compression);
            let mut in_buf = fhm.get_filehandle().await;
            self.load(&mut in_buf, &compression, start).await;
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

            let children = self
                .children
                .as_ref()
                .expect("Not a leaf but children are empty")
                .read()
                .await;

            if children[i].read().await.children_in_memory {
                let child = children[i].read().await;
                let compression = Arc::clone(&compression);
                return Box::pin(child.search_read(
                    fhm,
                    compression,
                    start,
                    key,
                ))
                .await;
            } else {
                let mut child = children[i].write().await;
                child.children_in_memory = child.children_loaded().await;
                let compression = Arc::clone(&compression);
                Box::pin(child.search(fhm, compression, start, key)).await
            }
        }
    }

    
    pub async fn search_read(
        &self,
        fhm: Arc<AsyncFileHandleManager>,
        compression: Arc<Option<CompressionConfig>>,
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

            let children = self
                .children
                .as_ref()
                .expect("Not a leaf but children are empty")
                .read()
                .await;

            if children[i].read().await.children_in_memory {
                let child = children[i].read().await;
                return Box::pin(child.search_read(
                    fhm,
                    compression,
                    start,
                    key,
                ))
                .await;
            } else {
                let mut child = children[i].write().await;
                child.children_in_memory = child.children_loaded().await;

                Box::pin(child.search(fhm, compression, start, key)).await
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

#[derive(Debug)]
#[cfg(feature = "async")]
pub struct AsyncFileHandleManager
{
    pub file_handles: Arc<RwLock<Vec<Arc<Mutex<BufReader<File>>>>>>,
    pub file_name: Option<String>,
}

#[cfg(feature = "async")]
impl Default for AsyncFileHandleManager
{
    fn default() -> Self
    {
        AsyncFileHandleManager {
            file_handles: Arc::new(RwLock::with_max_readers(Vec::new(), 128)),
            file_name: None,
        }
    }
}

#[cfg(feature = "async")]
impl AsyncFileHandleManager
{
    
    pub async fn get_filehandle(&self) -> OwnedMutexGuard<BufReader<File>>
    {
        let file_handles = Arc::clone(&self.file_handles);

        loop {
            let file_handles_read = file_handles.read().await;

            // Loop through each until we find one that is not locked
            for file_handle in file_handles_read.iter() {
                let file_handle = Arc::clone(file_handle);
                let file_handle = file_handle.try_lock_owned();
                if let Ok(file_handle) = file_handle {
                    return file_handle;
                }
            }

            if file_handles_read.len() < 8 {
                // There are no available file handles, so we need to create a
                // new one and the number isn't crazy (yet)
                break;
            }

            drop(file_handles_read);

            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }

        // Otherwise, create one and add it to the list
        let file = tokio::fs::File::open(self.file_name.as_ref().unwrap())
            .await
            .unwrap();

        #[cfg(unix)]
        {
            nix::fcntl::posix_fadvise(
                file.as_raw_fd(),
                0,
                0,
                nix::fcntl::PosixFadviseAdvice::POSIX_FADV_RANDOM,
            )
            .expect("Fadvise Failed");
        }

        let file_handle = std::sync::Arc::new(Mutex::new(
            tokio::io::BufReader::with_capacity(32 * 1024, file),
            // tokio::io::BufReader::new(file),
        ));

        let mut write_lock = file_handles.write().await;

        let cfh = Arc::clone(&file_handle);
        let fh = file_handle.try_lock_owned().unwrap();

        write_lock.push(cfh);

        fh
    }
}

/*

pub struct FractalTreeDiskAsyncIterator<K: Key, V: Value>
{
    tree: Arc<FractalTreeDiskAsync<K, V>>,
    current: usize,
    current_leaf: Option<Arc<RwLock<NodeDiskAsync<K, V>>>>,
}

impl<K: Key, V: Value> FractalTreeDiskAsyncIterator<K, V>
{
    pub async fn new(tree: Arc<FractalTreeDiskAsync<K, V>>) -> Self
    {

        // Get the first leaf node
        let root = tree.root.read().await;
        let mut current_leaf = root;
        while !current_leaf.is_leaf {
            let children = current_leaf.children.as_ref().unwrap().read().unwrap();
            current_leaf = children[0].read().unwrap();
        }


        FractalTreeDiskAsyncIterator { tree, current: 0 }
    }
}

impl<K: Key, V: Value> AsyncIterator for FractalTreeDiskAsyncIterator<K, V>
{
    type Item = (K, V);

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>>
    {

    }
}

*/