// This whole file is behind the async flag, so we don't need to worry
// about

use std::{
    async_iter::AsyncIterator,
    collections::BTreeMap,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use bincode::{BorrowDecode, Decode};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncSeekExt, BufReader, SeekFrom},
    sync::{Mutex, OwnedMutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use async_stream::stream;
use tokio_stream::Stream;

#[cfg(unix)]
use std::os::fd::AsRawFd;

use crate::*;
use libcompression::*;

/// This is the on-disk version of the FractalTree
///
/// The root node is loaded with the fractal tree, but children are
/// loaded on demand
///
/// You should generally interact with ArcTreeDiskAsync rather than
/// the tree directly...
#[derive(Debug)]
pub struct FractalTreeDiskAsync<K: Key, V: Value>
{
    pub root: ArcNodeDiskAsync<K, V>,
    pub start: u64, /* On disk position of the fractal tree, such that
                     * all locations are start + offset */
    pub compression: Arc<Option<CompressionConfig>>,
    pub file: Option<String>,
    pub file_handle_manager: Arc<AsyncFileHandleManager>,

    // Which nodes have been opened, in case they haven't been placed on the
    // tree yet such as from a DFS search and "right" thing
    pub opened: Arc<RwLock<BTreeMap<u64, ArcNodeDiskAsync<K, V>>>>,
}

impl<K: Key, V: Value> Decode for FractalTreeDiskAsync<K, V>
{
    fn decode<D: bincode::de::Decoder>(
        decoder: &mut D,
    ) -> core::result::Result<Self, bincode::error::DecodeError>
    {
        let start: u64 = bincode::Decode::decode(decoder)?;
        let compression: Option<CompressionConfig> =
            bincode::Decode::decode(decoder)?;
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
            root: root.into(),
            start,
            compression,
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
            root: NodeDiskAsync {
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
                parent: None,
                position_in_parent: 0,
            }
            .into(),
            start: 0,
            compression: Arc::new(None), // Default to no compression
            file: None,
            file_handle_manager: Default::default(),
            opened: Arc::new(RwLock::new(BTreeMap::new())),
        }
    }
}

impl<K: Key, V: Value> FractalTreeDiskAsync<K, V>
{
    /// Iterator through all leaves, in key order
    pub async fn stream(self: Arc<Self>) -> impl Stream<Item = (K, V)>
    {
        let gen = stream! {
            let mut current_leaf_idx = 0;
            self.load_all_leaves().await.unwrap();

            let keys = self.opened.read().await.keys().cloned().collect::<Vec<u64>>();

            if self.root.read().await.is_leaf {
                let root = self.root.read().await;
                for i in 0..root.keys.len() {
                    yield (root.keys[i].clone(), root.values.as_ref().unwrap()[i].clone());
                }
                return;
            }

            log::trace!("Current Index: {} / Keys Len: {}", current_leaf_idx, keys.len());

            while current_leaf_idx < keys.len() {
                let key = keys[current_leaf_idx].clone();
                let read_handle = self.opened.read().await;
                let node = read_handle.get(&key).unwrap().read().await;

                // Confirm node is a leaf, if not, iterator is done
                if !node.is_leaf {
                    break;
                }

                for i in 0..node.keys.len() {
                    log::trace!("Yielding key: {:?}", node.keys[i]);
                    yield (node.keys[i].clone(), node.values.as_ref().unwrap()[i].clone());
                }

                current_leaf_idx += 1;
            }
        };

        gen

    }

    /// Loads the entire tree into memory
    pub async fn load_tree(self: Arc<Self>) -> Result<(), &'static str>
    {
        // Root is always loaded, so can skip that
        // If root is a leaf, we're done
        let root = self.root.read().await;
        if root.is_leaf {
            return Ok(());
        }

        // Load the children of the root
        self.load_children(self.root.clone()).await?;

        // Recursively load the children of the children
        // this isn't recursive....
        let children = root.children.as_ref().unwrap().read().await;
        for child in children.iter() {
            self.load_children(child.clone()).await?;

            // If the child is not a leaf, we need to load its children
            let child_r = child.read().await;
            if !child_r.is_leaf {
                drop(child_r);
                self.load_children(child.clone()).await?;
            }
        }

        Ok(())
    }

    // todo this should run async in the background, i.e., once one is loaded should start displaying
    // data / sequence!!!
    pub async fn load_all_leaves(self: &Arc<Self>) -> Result<(), &'static str>
    {

        if self.root.read().await.is_leaf {
            return Ok(());
        }

        // Get lock on self.opened
        let mut opened = self.opened.write().await;

        // Clear the opened nodes, just to prevent any funkiness if anyone
        // keeps it open a long time (shouldn't happen)
        opened.clear();
        drop(opened);

        // Get filehandle
        let mut in_buf = self.file_handle_manager.get_filehandle().await;

        in_buf.seek(SeekFrom::Start(self.start)).await.unwrap();

        log::debug!("Opening first node");

        let mut node: Arc<RwLock<NodeDiskAsync<K, V>>> =
            Arc::new(RwLock::new(NodeDiskAsync::from_loc(0)));

        let result = Arc::clone(&self).load_node_next_fh(node.clone(), &mut in_buf).await;

        match result {
            Ok(_) => (),
            Err(_) => return Err("Failed to load first node"),
        }

        log::debug!("First node opened");

        while node.read().await.is_leaf {
            let num_keys = node.read().await.keys.len();
            log::trace!("Is leaf, reading next one - this one has {} keys", num_keys);
            // Read the next one
            let pos = in_buf.stream_position().await.unwrap();
            log::trace!("Current position: {:?}", pos);
            node = Arc::new(RwLock::with_max_readers(
                NodeDiskAsync::<K, V>::from_loc(0), // Doesn't matter
                128,
            ));

            let borrowed_self = Arc::clone(&self);
            let borrowed_node = Arc::clone(&node);
            let result = borrowed_self.load_node_next_fh(borrowed_node, &mut in_buf).await;

            match result {
                Ok(_) => (),
                Err(_) => {
                    // If we loaded some nodes, this isn't an error
                    if self.opened.read().await.len() > 0 {
                        return Ok(());
                    } else {
                        return Err("Failed to load next node");
                    }
                },
            }

            // let node_clone = Arc::clone(&node);
            // let node_clone: ArcNodeDiskAsync<K, V> = node_clone.into();
            // opened.insert(pos, node_clone);
        }

        log::trace!("No longer reading leaves");
        Ok(())
    }

    pub async fn load_children(
        self: &Arc<Self>,
        node: ArcNodeDiskAsync<K, V>,
    ) -> Result<(), &'static str>
    {
        let node_read = node.read().await;

        if node_read.children_loaded().await {
            return Ok(());
        }

        let children = node_read.children.as_ref().unwrap().read().await;

        let mut handles = Vec::new();

        for child in children.iter() {
            let child = Arc::clone(child.into());
            let tree = Arc::clone(self);
            handles.push(tokio::spawn(async move {
                tree.load_node(child.into()).await
            }));
        }

        for handle in handles {
            handle.await.expect("Failed to load node");
        }

        // Populate the left and right pointers (where we can)
        // Only if children are leaves
        let test_child = children[0].read().await;

        let populate = test_child.is_leaf;
        drop(test_child);

        // Populate the left and right pointers (if leaves)
        // Also assign the parent and position_in_parent
        for (i, child) in children.iter().enumerate() {
            let mut child = child.write().await;

            // Parent and position allow us to better navigate the tree
            // but don't need to be stored in a file
            child.parent = Some(node.arc_sugar());
            child.position_in_parent = i as u32;

            if populate {
                if i > 0 {
                    child.left = Some(children[i - 1].arc_sugar());
                }

                if i < children.len() - 1 {
                    child.right = Some(children[i + 1].arc_sugar());
                }
            }
        }

        Ok(())
    }

    pub async fn len(self: &Arc<Self>) -> Result<usize, &'static str>
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
        self.root
            .search(
                Arc::clone(&self.file_handle_manager),
                Arc::clone(&self.compression),
                self.start,
                key,
            )
            .await
    }

    // Find the first leaf of the node
    // pub async fn first_leaf(
    // &self,
    // ) -> Arc<RwLock<NodeDiskAsync<K, V>>>
    // {
    // let root = self.root.read().await;
    // let mut current_leaf = root;
    // while !current_leaf.is_leaf {
    // Load node if it's not in memory
    // if current_leaf.state.on_disk() {
    // let mut in_buf = self.file_handle_manager.get_filehandle().await;
    // current_leaf.load(&mut in_buf, &self.compression,
    // self.start).await; }
    // let children =
    // current_leaf.children.as_ref().unwrap().read().await;
    // current_leaf = children[0].read().await;
    // }
    //
    // current_leaf
    // }

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
                { 8 * 1024 },
                _,
                _,
            >(&mut in_buf, bincode_config)
            .await
            {
                Ok(x) => x,
                Err(_) => {
                    return Result::Err("Failed to decode FractalTreeDiskAsync")
                }
            };

        tree.file = Some(file);
        tree.file_handle_manager = Arc::new(fhm);

        Ok(tree)
    }

    pub async fn load_node(
        self: Arc<Self>,
        node: Arc<RwLock<NodeDiskAsync<K, V>>>,
    )
    {
        let mut node_write = node.write().await;

        if !node_write.state.on_disk() {
            return;
        }

        let mut in_buf = self.file_handle_manager.get_filehandle().await;

        in_buf
            .seek(SeekFrom::Start(self.start + node_write.loc))
            .await
            .unwrap();

        let loc = node_write.loc;

        let config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<{ 8 * 1024 * 1024 }>();

        let mut loaded_node: NodeDiskAsync<K, V> = if self.compression.is_some()
        {
            let compressed: Vec<u8> =
                bincode_decode_from_buffer_async_with_size_hint::<
                    { 8 * 1024 },
                    _,
                    _,
                >(&mut in_buf, config)
                .await
                .unwrap();

            let compression = Arc::clone(&self.compression);
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
            bincode_decode_from_buffer_async_with_size_hint::<{8 * 1024}, _, _>(&mut in_buf, config).await.unwrap()
        };

        delta_decode(&mut loaded_node.keys);
        loaded_node.loc = loc;
        *node_write = loaded_node;

        // Release the lock
        drop(node_write);

        let mut opened = self.opened.write().await;

        log::trace!("Inserting node into opened: {:?}", loc);

        opened.insert(loc, Arc::clone(&node).into());
    }

    pub async fn load_node_next_fh(
        self: Arc<Self>,
        node: Arc<RwLock<NodeDiskAsync<K, V>>>,
        mut in_buf: &mut OwnedMutexGuard<BufReader<File>>,
    ) -> Result<(), &'static str>
    {
        let mut node_write = node.write().await;

        if !node_write.state.on_disk() {
            return Ok(());
        }

        let loc = in_buf.stream_position().await.unwrap();
        log::trace!("Loading node at: {:?}", loc);

        let config = bincode::config::standard()
            .with_variable_int_encoding()
            .with_limit::<{ 1024 * 1024 }>();

        let mut loaded_node: NodeDiskAsync<K, V> = if self.compression.is_some()
        {
            let compressed: Vec<u8> =
                match bincode_decode_from_buffer_async_with_size_hint::<
                    { 32 * 1024 },
                    _,
                    _,
                >(&mut in_buf, config)
                .await {
                    Ok(x) => x,
                    Err(_) => {
                        return Err("Failed to decode compressed node");
                    }
                };
                

            log::trace!("Compressed Node: {:?}", compressed.len());

            log::trace!("Stream position now at {:?}", in_buf.stream_position().await.unwrap());

            let compression = Arc::clone(&self.compression);
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

        log::trace!("Inserting node into opened: {:?}", loc);

        opened.insert(loc, Arc::clone(&node).into());
        
        Ok(())

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
    pub state: NodeStateAsync,
    pub keys: Vec<K>,
    pub parent: Option<ArcNodeDiskAsync<K, V>>,
    pub children: Option<Arc<RwLock<Vec<ArcNodeDiskAsync<K, V>>>>>,
    pub values: Option<Vec<V>>,
    pub left: Option<ArcNodeDiskAsync<K, V>>,
    pub right: Option<ArcNodeDiskAsync<K, V>>,
    pub loc: u64,
    pub position_in_parent: u32,
    pub is_root: bool,
    pub is_leaf: bool,
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
            let children: Vec<ArcNodeDiskAsync<K, V>> = locs
                .iter()
                .map(|x| {
                    {
                        Arc::new(RwLock::with_max_readers(
                            NodeDiskAsync::<K, V>::from_loc(*x),
                            128,
                        ))
                    }
                    .into()
                })
                .collect::<Vec<ArcNodeDiskAsync<K, V>>>();
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
            parent: None,
            position_in_parent: 0,
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
            let children: Vec<ArcNodeDiskAsync<K, V>> = locs
                .iter()
                .map(|x| {
                    {
                        Arc::new(RwLock::with_max_readers(
                            NodeDiskAsync::<K, V>::from_loc(*x),
                            128,
                        ))
                    }
                    .into()
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
            parent: None,
            position_in_parent: 0,
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
            parent: None,
            position_in_parent: 0,
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

                for child in children.iter() {
                    let child = child.clone();
                    let fhm = Arc::clone(&fhm);
                    let compression = Arc::clone(&compression);
                    handles.push(tokio::spawn(async move {
                        let mut child = child.0.write_owned().await;
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

    let start_pos = in_buf.stream_position().await.unwrap();

    let mut buf = vec![0; SIZE_HINT];
    match in_buf.read(&mut buf).await {
        Ok(_) => (),
        Err(_) => return Result::Err("Failed to read buffer".to_string()),
    }

    loop {
        match bincode::decode_from_slice(&buf, bincode_config) {
            Ok((_, 0)) => (),
            Ok((x, size)) => {

                log::trace!("Decoded: {} bytes", size);
                in_buf.seek(SeekFrom::Start(start_pos + size as u64)).await.unwrap();
                log::trace!("Seeking back to {:?}", start_pos + size as u64);

                
                return Ok(x);
            }
            Err(_) => (),
        };

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

// pub struct FractalTreeDiskAsyncIterator<K: Key, V: Value>
// {
// tree: Arc<FractalTreeDiskAsync<K, V>>,
// current: usize,
// current_leaf: Option<Arc<RwLock<NodeDiskAsync<K, V>>>>,
// }
//
// impl<K: Key, V: Value> FractalTreeDiskAsyncIterator<K, V>
// {
// pub async fn new(tree: Arc<FractalTreeDiskAsync<K, V>>) -> Self
// {
//
// Get the first leaf node
// let root = tree.root.read().await;
// let mut current_leaf = root;
// while !current_leaf.is_leaf {
// let children =
// current_leaf.children.as_ref().unwrap().read().unwrap();
// current_leaf = children[0].read().unwrap();
// }
//
//
// FractalTreeDiskAsyncIterator { tree, current: 0 }
// }
// }
//
// impl<K: Key, V: Value> AsyncIterator for
// FractalTreeDiskAsyncIterator<K, V> {
// type Item = (K, V);
//
// fn poll_next(
// mut self: Pin<&mut Self>,
// cx: &mut Context<'_>,
// ) -> Poll<Option<Self::Item>>
// {
//
// }
// }
//

// This is ugly so the rest can be look a little better
#[derive(Debug, Clone)]
pub struct ArcNodeDiskAsync<K, V>(Arc<RwLock<NodeDiskAsync<K, V>>>);

impl<K: Key, V: Value> ArcNodeDiskAsync<K, V>
{
    pub async fn search(
        &self,
        fhm: Arc<AsyncFileHandleManager>,
        compression: Arc<Option<CompressionConfig>>,
        start: u64,
        key: &K,
    ) -> Option<V>
    {
        let node = Arc::clone(self.into());
        let node = node.read_owned().await;
        node.search_read(fhm, compression, start, key).await
    }

    pub fn arc(&self) -> Arc<RwLock<NodeDiskAsync<K, V>>>
    {
        Arc::clone(&self.0)
    }

    pub fn arc_sugar(&self) -> ArcNodeDiskAsync<K, V>
    {
        ArcNodeDiskAsync(Arc::clone(&self.0))
    }

    pub async fn read(&self) -> RwLockReadGuard<NodeDiskAsync<K, V>>
    {
        self.0.read().await
    }

    pub async fn write(&self) -> RwLockWriteGuard<NodeDiskAsync<K, V>>
    {
        self.0.write().await
    }
}

impl<K: Key, V: Value> From<ArcNodeDiskAsync<K, V>>
    for Arc<RwLock<NodeDiskAsync<K, V>>>
{
    fn from(node: ArcNodeDiskAsync<K, V>) -> Self
    {
        node.0
    }
}

impl<K: Key, V: Value> From<Arc<RwLock<NodeDiskAsync<K, V>>>>
    for ArcNodeDiskAsync<K, V>
{
    fn from(node: Arc<RwLock<NodeDiskAsync<K, V>>>) -> Self
    {
        ArcNodeDiskAsync(node)
    }
}

impl<'a, K: Key, V: Value> From<&'a ArcNodeDiskAsync<K, V>>
    for &'a Arc<RwLock<NodeDiskAsync<K, V>>>
{
    fn from(node: &'a ArcNodeDiskAsync<K, V>) -> Self
    {
        &node.0
    }
}

impl<K: Key, V: Value> From<NodeDiskAsync<K, V>> for ArcNodeDiskAsync<K, V>
{
    fn from(node: NodeDiskAsync<K, V>) -> Self
    {
        ArcNodeDiskAsync(Arc::new(RwLock::with_max_readers(node, 128)))
    }
}

// Same for the tree
#[derive(Debug)]
pub struct ArcTreeDiskAsync<K: Key, V: Value>(Arc<FractalTreeDiskAsync<K, V>>);

impl<K: Key, V: Value> ArcTreeDiskAsync<K, V>
{
    pub fn arc(&self) -> Arc<FractalTreeDiskAsync<K, V>>
    {
        Arc::clone(&self.0)
    }

    pub fn as_arc(&self) -> &Arc<FractalTreeDiskAsync<K, V>>
    {
        &self.0
    }
}

impl<K: Key, V: Value> From<Arc<FractalTreeDiskAsync<K, V>>>
    for ArcTreeDiskAsync<K, V>
{
    fn from(tree: Arc<FractalTreeDiskAsync<K, V>>) -> Self
    {
        ArcTreeDiskAsync(tree)
    }
}

impl<K: Key, V: Value> From<ArcTreeDiskAsync<K, V>>
    for Arc<FractalTreeDiskAsync<K, V>>
{
    fn from(tree: ArcTreeDiskAsync<K, V>) -> Self
    {
        tree.0
    }
}

impl<'a, K: Key, V: Value> From<&'a ArcTreeDiskAsync<K, V>>
    for &'a Arc<FractalTreeDiskAsync<K, V>>
{
    fn from(tree: &'a ArcTreeDiskAsync<K, V>) -> Self
    {
        &tree.0
    }
}

impl<K: Key, V: Value> From<FractalTreeDiskAsync<K, V>>
    for ArcTreeDiskAsync<K, V>
{
    fn from(tree: FractalTreeDiskAsync<K, V>) -> Self
    {
        ArcTreeDiskAsync(Arc::new(tree))
    }
}
