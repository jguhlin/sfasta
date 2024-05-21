use super::*;
use sorted_vec::SortedVec;

/// This is an insertion-only B+ tree, deletions are not supported
/// Meant for a write-many, write-once to disk, read-only-and-many
/// database
///
/// first_leaf is the stored location of the first leaf, which itself
/// has the address to the node to the right
#[derive(Debug)]
pub struct FractalTreeBuild<K: Key, V: Value>
{
    pub root: NodeBuild<K, V>,
    pub order: usize,
    pub buffer_size: usize,
}

impl<K: Key, V: Value> FractalTreeBuild<K, V>
{
    pub fn new(order: usize, buffer_size: usize) -> Self
    {
        let mut root = NodeBuild::leaf(order, buffer_size);
        root.is_root = true;
        FractalTreeBuild {
            root,
            order,
            buffer_size,
        }
    }

    pub fn insert(&mut self, key: K, value: V)
    {
        if self.root.insert(self.buffer_size, key, value) {
            self.flush(false);
        }
    }

    pub fn flush(&mut self, all: bool)
    {
        self.root.flush(self.order, self.buffer_size, all);

        while self.root.needs_split(self.order) {
            let (new_key, mut new_node) =
                self.root.split(self.order, self.buffer_size);
            let old_root = NodeBuild::internal(self.order, self.buffer_size);
            let mut old_root = Box::new(old_root);
            old_root.is_root = true;
            self.root.is_root = false;
            new_node.is_root = false;

            std::mem::swap(&mut self.root, &mut old_root);

            self.root.keys.push(new_key);
            self.root.children.as_mut().unwrap().push(old_root);
            self.root.children.as_mut().unwrap().push(new_node);
        }

        self.root.flush(self.order, self.buffer_size, all);
    }

    pub fn flush_all(&mut self)
    {
        self.flush(true);
    }

    pub fn search(&self, key: &K) -> Option<V>
    {
        self.root.search(&key)
    }

    // Gets the depth by following the first child, iteratively, until
    // it's a leaf
    pub fn depth(&self) -> usize
    {
        let mut depth = 0;
        let mut node = &self.root;
        while !node.is_leaf {
            node = &node.children.as_ref().unwrap()[0];
            depth += 1;
        }
        depth
    }

    pub fn count_all_nodes(&self) -> usize
    {
        let mut count = 0;
        let mut stack = vec![&self.root];
        while let Some(node) = stack.pop() {
            count += 1;
            if !node.is_leaf {
                for child in node.children.as_ref().unwrap().iter() {
                    stack.push(child);
                }
            }
        }
        count
    }
}

#[derive(Debug)]
pub struct NodeBuild<K: Key, V: Value>
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub keys: SortedVec<K>,
    pub children: Option<Vec<Box<NodeBuild<K, V>>>>,
    pub values: Option<Vec<V>>,
    pub buffer: Vec<(K, V)>,
}

impl<K: Key, V: Value> NodeBuild<K, V>
{
    pub fn internal(order: usize, buffer_size: usize) -> Self
    {
        NodeBuild {
            is_root: false,
            is_leaf: false,
            keys: SortedVec::with_capacity(order),
            children: Some(Vec::with_capacity(order)),
            values: None,
            buffer: Vec::with_capacity(buffer_size),
        }
    }

    pub fn leaf(order: usize, buffer_size: usize) -> Self
    {
        NodeBuild {
            is_root: false,
            is_leaf: true,
            keys: SortedVec::with_capacity(order),
            children: None,
            values: Some(Vec::with_capacity(order)),
            buffer: Vec::with_capacity(buffer_size),
        }
    }

    pub fn search(&self, key: &K) -> Option<V>
    {
        debug_assert!(self.buffer.is_empty());

        let i = self.keys.binary_search(&key);

        if self.is_leaf {
            let i = match i {
                Ok(i) => i,
                Err(_) => return None, /* This is the leaf, if it's not found
                                        * here it won't be found */
            };

            debug_assert!(i < self.values.as_ref().unwrap().len());
            Some(self.values.as_ref().unwrap()[i].clone())
        } else {
            let i = match i {
                Ok(i) => i.saturating_add(1),
                Err(i) => i,
            };

            self.children.as_ref().unwrap()[i].search(key)
        }
    }

    pub fn insert(&mut self, buffer_size: usize, key: K, value: V) -> bool
    {
        self.buffer.push((key, value));
        self.buffer.len() >= buffer_size
    }

    pub fn flush(&mut self, order: usize, buffer_size: usize, all: bool)
    {
        // This flushes the buffer down the tree, or if a leaf node, into the
        // tree

        if self.is_leaf {
            self.keys.reserve(self.buffer.len());
            self.values.as_mut().unwrap().reserve(self.buffer.len());
            for (key, value) in self.buffer.drain(..) {
                let i = self.keys.insert(key);
                self.values.as_mut().unwrap().insert(i, value);
            }
        } else {
            for (key, value) in self.buffer.drain(..) {
                let i = match self.keys.binary_search(&key) {
                    Ok(i) => i,
                    Err(i) => i,
                };
                // Insert into child node
                let result = self.children.as_mut().unwrap()[i].insert(
                    buffer_size,
                    key,
                    value,
                );

                if result {
                    self.children.as_mut().unwrap()[i].flush(
                        order,
                        buffer_size,
                        false,
                    );
                }
            }
        }

        if !self.is_leaf {
            // While any children need to be split...

            while self
                .children
                .as_ref()
                .unwrap()
                .iter()
                .any(|child| child.needs_split(order))
            {
                for child_i in 0..self.children.as_ref().unwrap().len() {
                    while self.children.as_mut().unwrap()[child_i]
                        .needs_split(order)
                    {
                        let (new_key, new_node) =
                            self.children.as_mut().unwrap()[child_i]
                                .split(order, buffer_size);
                        let i = self.keys.insert(new_key) + 1;
                        if i >= self.children.as_ref().unwrap().len() {
                            self.children.as_mut().unwrap().push(new_node);
                        } else {
                            self.children.as_mut().unwrap().insert(i, new_node);
                        }
                    }
                }
            }
        }

        if all && !self.is_leaf {
            for child_i in 0..self.children.as_ref().unwrap().len() {
                self.children.as_mut().unwrap()[child_i].flush(
                    order,
                    buffer_size,
                    all,
                );
            }
        }

        if all && self.buffer.len() > 0 {
            panic!("Buffer not empty!");
        }
    }

    pub fn split(
        &mut self,
        order: usize,
        buffer_size: usize,
    ) -> (K, Box<NodeBuild<K, V>>)
    {
        debug_assert!(self.keys.is_sorted());

        self.flush(order, buffer_size, true);

        let mid = self.keys.len() / 2;

        let values = if self.values.is_some() {
            let mut values = self.values.as_mut().unwrap().split_off(mid);
            self.values.as_mut().unwrap().reserve(values.len());
            values.reserve(values.len());
            Some(values)
        } else {
            None
        };

        let children: Option<Vec<Box<NodeBuild<K, V>>>> =
            if self.children.is_some() {
                let mut children =
                    self.children.as_mut().unwrap().split_off(mid + 1);
                self.children.as_mut().unwrap().reserve(children.len());
                children.reserve(children.len());
                Some(children)
            } else {
                None
            };

        debug_assert!(mid < self.keys.len());

        let keys = self.keys.split_at(mid);
        let mut orig_keys = unsafe { SortedVec::from_sorted(keys.0.to_vec()) };
        let mut keys = unsafe { SortedVec::from_sorted(keys.1.to_vec()) };

        orig_keys.reserve(keys.len());
        keys.reserve(keys.len());

        self.keys = orig_keys;

        let mut new_node = Box::new(NodeBuild {
            is_root: false,
            is_leaf: self.is_leaf,
            buffer: Vec::with_capacity(self.buffer.capacity()),
            keys,
            children,
            values, /* next: None, // TODO
                     * next: self.next.take(), */
        });

        let new_key = if self.is_leaf {
            new_node.keys[0].clone()
        } else {
            new_node.keys.remove_index(0)
        };

        (new_key, new_node)
    }

    pub fn needs_split(&self, order: usize) -> bool
    {
        // >= because buffering
        self.keys.len() >= order
    }
}

#[cfg(test)]
mod tests
{
    use super::*;
    use bincode::config;
    use human_size::SpecificSize;
    use rand::prelude::*;
    use xxhash_rust::xxh3::xxh3_64;

    #[test]
    fn split()
    {
        let mut node: NodeBuild<u32, u32> = super::NodeBuild {
            is_root: false,
            is_leaf: true,
            keys: unsafe {
                SortedVec::from_sorted(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11])
            },
            children: None,
            values: Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
            buffer: Vec::with_capacity(8),
        };

        // Initial node: keys: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        // After split:
        // Node: keys: [1, 2, 3, 4, 5], values: [1, 2, 3, 4, 5]
        // New Node: keys: [6, 7, 8, 9, 10, 11], values: [6, 7, 8, 9, 10, 11]
        // New key: 6

        let (new_key, new_node) = node.split(2, 2);
        assert_eq!(new_key, 6);
        assert_eq!(new_node.keys, unsafe {
            SortedVec::from_sorted(vec![6, 7, 8, 9, 10, 11])
        });
        assert_eq!(new_node.values, Some(vec![6, 7, 8, 9, 10, 11]));
        assert_eq!(node.keys, unsafe {
            SortedVec::from_sorted(vec![1, 2, 3, 4, 5])
        });
        assert_eq!(node.values, Some(vec![1, 2, 3, 4, 5]));

        let mut node: NodeBuild<u32, u32> = super::NodeBuild {
            is_root: false,
            is_leaf: true,
            keys: SortedVec::from_unsorted((0..28).collect()),
            children: None,
            values: Some((0..28).collect()),
            buffer: Vec::with_capacity(8),
        };

        let (new_key, new_node) = node.split(2, 2);
        assert!(new_key == 14);
        assert_eq!(
            new_node.keys,
            SortedVec::from_unsorted((14..28).collect::<Vec<_>>())
        );
        assert_eq!(new_node.values, Some((14..28).collect::<Vec<_>>()));
        assert_eq!(node.keys.to_vec(), (0..14).collect::<Vec<_>>());
        assert_eq!(node.values, Some((0..14).collect::<Vec<_>>()));

        let (new_key, new_node_) = node.split(2, 2);
        assert!(new_key == 7);
        assert_eq!(new_node_.keys.to_vec(), (7..14).collect::<Vec<_>>());
        assert_eq!(new_node_.values, Some((7..14).collect::<Vec<_>>()));
        assert_eq!(node.keys.to_vec(), (0..7).collect::<Vec<_>>());
        assert_eq!(node.values, Some((0..7).collect::<Vec<_>>()));
    }

    #[test]
    fn basic_tree()
    {
        let mut tree = super::FractalTreeBuild::new(6, 3);
        tree.insert(0, 0);

        let mut rng = thread_rng();
        let mut values = (0..1024_u32).collect::<Vec<u32>>();
        values.shuffle(&mut rng);

        for i in values.iter() {
            tree.insert(*i, *i as u32);
        }

        assert!(!tree.root.is_leaf);
    }

    #[test]
    fn simple_insertions()
    {
        let mut tree: FractalTreeBuild<u32, u32> =
            super::FractalTreeBuild::new(6, 3);
        tree.insert(1, 1);
        tree.insert(2, 2);
        tree.insert(3, 3);
        tree.insert(4, 4);
        tree.insert(5, 5);
        tree.insert(6, 6);
        tree.insert(7, 7);
        tree.insert(8, 8);
        tree.insert(9, 9);
        tree.insert(10, 10);
    }

    #[test]
    fn tree_structure()
    {
        let mut tree = super::FractalTreeBuild::new(8, 3);

        let mut rng = thread_rng();
        let mut values = (0..1024_u32).collect::<Vec<u32>>();
        values.shuffle(&mut rng);

        for i in values.iter() {
            tree.insert(*i, *i as u32);
        }

        // Iterate through the tree, all leaves should have keys.len() ==
        // vales.len() All internal nodes should have keys.len() ==
        // children.len() - 1
        let mut stack = vec![&tree.root];
        while let Some(node) = stack.pop() {
            if node.is_leaf {
                assert_eq!(
                    node.keys.len(),
                    node.values.as_ref().unwrap().len()
                );
            } else {
                assert_eq!(
                    node.keys.len() + 1,
                    node.children.as_ref().unwrap().len(),
                    "Node: {:#?}",
                    node
                );
                for child in node.children.as_ref().unwrap().iter() {
                    stack.push(child);
                }
            }
        }

        // Check that the keys are ordered
        let mut stack = vec![&tree.root];
        while let Some(node) = stack.pop() {
            if node.is_leaf {
                for i in 1..node.keys.len() {
                    assert!(node.keys[i - 1] < node.keys[i]);
                }
            } else {
                for i in 1..node.keys.len() {
                    assert!(node.keys[i - 1] < node.keys[i]);
                }
                for child in node.children.as_ref().unwrap().iter() {
                    stack.push(child);
                }
            }
        }
    }

    #[test]
    fn search()
    {
        let mut rng = thread_rng();
        let mut values_1024 = (0..1024_u32).collect::<Vec<u32>>();
        values_1024.shuffle(&mut rng);

        let mut values_8192 = (0..8192_u32).collect::<Vec<u32>>();
        values_8192.shuffle(&mut rng);

        let mut tree = super::FractalTreeBuild::new(8, 8);

        for i in values_1024.iter() {
            tree.insert(*i, *i as u32);
        }

        tree.flush_all();

        for i in values_1024.iter() {
            assert_eq!(tree.search(i), Some(*i as u32));
        }

        let mut tree = super::FractalTreeBuild::new(16, 32);

        for i in values_8192.iter() {
            tree.insert(*i, *i as u32);
        }

        tree.flush_all();

        for i in values_8192.iter() {
            assert_eq!(tree.search(i), Some(*i as u32));
        }

        // Find value does not exist
        assert_eq!(tree.search(&8192), None);

        let mut tree = super::FractalTreeBuild::new(64, 32);
        for i in 0..(1024 * 1024_u32) {
            tree.insert(i as u32, i as u32);
        }

        tree.flush_all();

        for i in 0..(1024 * 1024_u32) {
            assert!(tree.search(&i) == Some(i as u32), "i: {}", i);
        }

        // Things that should not be found
        assert!(tree.search(&(1024 * 1024_u32)) == None);
        assert!(tree.search(&(1024 * 1024_u32 + 1)) == None);

        // New tree
        let mut tree = super::FractalTreeBuild::new(8, 4);
        for i in 1024..2048_u32 {
            tree.insert(i, i as u32);
        }

        tree.flush_all();

        for i in 1024..2048_u32 {
            assert_eq!(tree.search(&i), Some(i as u32));
        }

        // Things that should not be found
        for i in 0..1024_u32 {
            assert_eq!(tree.search(&i), None);
        }
        for i in 2048..4096_u32 {
            assert_eq!(tree.search(&i), None);
        }
    }

    #[cfg(not(feature = "async"))]
    #[test]
    fn search_noderead()
    {
        let mut rng = thread_rng();
        let mut values_1024 = (0..1024_u32).collect::<Vec<u32>>();
        values_1024.shuffle(&mut rng);

        let mut values_8192 = (0..8192_u32).collect::<Vec<u32>>();
        values_8192.shuffle(&mut rng);

        let mut tree = super::FractalTreeBuild::new(32, 8);

        for i in values_1024.iter() {
            tree.insert(*i, *i as u32);
        }

        tree.flush_all();

        let mut tree = FractalTreeDisk::from(tree);
        let mut buf = std::io::Cursor::new(Vec::new());

        for i in values_1024.iter() {
            assert_eq!(tree.search(&mut buf, &i), Some(*i));
        }
    }

    #[ignore]
    #[test]
    fn size_of_super_large()
    {
        let values128m = (0..128_369_206_u32)
            .map(|x| xxh3_64(&x.to_le_bytes()))
            // Grab the lower bits as a u32
            .map(|x| x as u32)
            .collect::<Vec<u32>>();

        let mut tree: FractalTreeBuild<u32, u32> =
            FractalTreeBuild::new(128, 256);
        for i in values128m.iter() {
            tree.insert(*i, *i as u32);
        }
        tree.flush_all();

        let depth = tree.depth();
        let node_count = tree.count_all_nodes();

        let tree: FractalTreeDisk<u32, u32> = tree.into();

        let config = config::standard();

        // TODO
        // Encoding is not implemented yet...
        // Doing a custom method...

        // let encoded: Vec<u8> = bincode::encode_to_vec(&tree,
        // config).unwrap(); println!("Size of 128m tree: {}",
        // encoded.len());

        // let size2: SpecificSize<human_size::Gigabyte> =
        // format!("{} B", encoded.len()).parse().unwrap();
        //
        // println!("Size of 128m tree: {}", size2);
        //
        // println!("Depth of 128m tree: {}", depth);
        // println!("Node count of 128m tree: {}", node_count);

        panic!();
    }

    #[test]
    fn tree_build_and_find()
    {
        use std::io::{Seek, SeekFrom};

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
        tree.flush_all();

        let result = tree.search(&1);

        assert!(result.is_some());
        assert_eq!(result.unwrap(), 1);
    }
}
