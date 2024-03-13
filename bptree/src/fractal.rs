// This is a derivation of the SortedVec tree (fastest, so far)
// Fractal adds a buffer so that insertions etc... are done in batches, rather than immediately
// Todo:
// Storable on disk
// Able to load only part of the tree from disk

// Notes:
// [x] Look into eytzinger (instead of sorted vec?) or ordsearch? -- Neither worked well here

// use bumpalo::Bump;
// use eytzinger::*;
use sorted_vec::SortedVec;
use bincode::{config, Decode, Encode};
// use pulp::Arch;

// This is an insertion-only B+ tree, deletions are simply not supported
// Meant for a write-many, write-once to disk, read-only-and-many database

#[derive(Debug)]
pub struct FractalTree<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    root: Node<K, V>,
    order: usize,
    buffer_size: usize,
}

impl<K, V> FractalTree<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    pub fn new(order: usize, buffer_size: usize) -> Self {
        let mut root = Node::leaf(order, buffer_size);
        root.is_root = true;
        FractalTree {
            root,
            order,
            buffer_size,
        }
    }

    pub fn insert(&mut self, key: K, value: V) {
        if self.root.insert(self.buffer_size, key, value) {
            self.flush(false);
        }
    }

    pub fn flush(&mut self, all: bool)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug + Copy,
    {
        assert!(self.root.keys.is_sorted());
        self.root.flush(self.order, self.buffer_size, all);

        while self.root.needs_split(self.order) {
            let (new_key, mut new_node) = self.root.split(self.order, self.buffer_size);
            let old_root = Node::internal(self.order, self.buffer_size);
            let mut old_root = Box::new(old_root);
            old_root.is_root = true;
            self.root.is_root = false;
            new_node.is_root = false;

            std::mem::swap(&mut self.root, &mut old_root);

            self.root.keys.push(new_key);
            self.root.children.as_mut().unwrap().push(old_root);
            self.root.children.as_mut().unwrap().push(new_node);
        }
    }

    pub fn flush_all(&mut self)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug + Copy,
    {
        self.flush(true);
    }

    pub fn search(&self, key: K) -> Option<V>
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug + Copy,
    {
        self.root.search(key)
    }
}

// Must use
#[must_use]
pub enum InsertionAction<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    Success,
    NodeSplit(K, Box<Node<K, V>>),
}

#[derive(Debug)]
pub struct Node<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub keys: SortedVec<K>,
    pub children: Option<Vec<Box<Node<K, V>>>>,
    pub values: Option<Vec<V>>,
    pub buffer: Vec<(K, V)>,
}

impl<K, V> Node<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    pub fn internal(order: usize, buffer_size: usize) -> Self {
        Node {
            is_root: false,
            is_leaf: false,
            keys: SortedVec::with_capacity(order),
            children: Some(Vec::with_capacity(order)),
            values: None,
            buffer: Vec::with_capacity(buffer_size),
        }
    }

    pub fn leaf(order: usize, buffer_size: usize) -> Self {
        Node {
            is_root: false,
            is_leaf: true,
            keys: SortedVec::with_capacity(order),
            children: None,
            values: Some(Vec::with_capacity(order)),
            buffer: Vec::with_capacity(buffer_size),
        }
    }

    pub fn search(&self, key: K) -> Option<V>
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug + Copy,
    {
        #[cfg(debug_assertions)]
        assert!(self.buffer.is_empty());

        let i = self.keys.binary_search(&key);

        if self.is_leaf {
            let i = match i {
                Ok(i) => i,
                Err(_) => return None, // This is the leaf, if it's not found here it won't be found
            };
            #[cfg(debug_assertions)]
            assert!(i < self.values.as_ref().unwrap().len());
            Some(self.values.as_ref().unwrap()[i])
        } else {
            let i = match i {
                Ok(i) => i + 1,
                Err(i) => i,
            };

            self.children.as_ref().unwrap()[i].search(key)
        }
    }

    pub fn insert(&mut self, buffer_size: usize, key: K, value: V) -> bool
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        self.buffer.push((key, value));
        self.buffer.len() >= buffer_size
    }

    pub fn flush(&mut self, order: usize, buffer_size: usize, all: bool) {
        // This flushes the buffer down the tree, or if a leaf node, into the tree

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
                let result = self.children.as_mut().unwrap()[i].insert(buffer_size, key, value);

                if result {
                    self.children.as_mut().unwrap()[i].flush(order, buffer_size, false);
                }
            }
        }

        if !self.is_leaf {
            for child_i in 0..self.children.as_ref().unwrap().len() {
                while self.children.as_mut().unwrap()[child_i].needs_split(order) {
                    let (new_key, new_node) =
                        self.children.as_mut().unwrap()[child_i].split(order, buffer_size);
                    let i = self.keys.insert(new_key) + 1;
                    if i >= self.children.as_ref().unwrap().len() {
                        self.children.as_mut().unwrap().push(new_node);
                    } else {
                        self.children.as_mut().unwrap().insert(i, new_node);
                    }
                }
            }
        }

        if all && !self.is_leaf {
            for child_i in 0..self.children.as_ref().unwrap().len() {
                self.children.as_mut().unwrap()[child_i].flush(order, buffer_size, all);
            }
        }

        if all && self.buffer.len() > 0 {
            panic!("Buffer not empty!");
        }
    }

    pub fn split(&mut self, order: usize, buffer_size: usize) -> (K, Box<Node<K, V>>)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        #[cfg(debug_assertions)]
        assert!(self.keys.is_sorted());

        self.flush(order, buffer_size, true);

        let mid = self.keys.len() / 2;

        let values = if self.values.is_some() {
            let values = self.values.as_mut().unwrap().split_off(mid);
            Some(values)
        } else {
            None
        };

        let children: Option<Vec<Box<Node<K, V>>>> = if self.children.is_some() {
            let children = self.children.as_mut().unwrap().split_off(mid + 1);
            Some(children)
        } else {
            None
        };

        #[cfg(debug_assertions)]
        assert!(mid < self.keys.len());
        let keys = self.keys.split_at(mid);
        let orig_keys = unsafe { SortedVec::from_sorted(keys.0.to_vec()) };
        let keys = unsafe { SortedVec::from_sorted(keys.1.to_vec()) };
        self.keys = orig_keys;

        let mut new_node = Box::new(Node {
            is_root: false,
            is_leaf: self.is_leaf,
            buffer: Vec::with_capacity(self.buffer.capacity()),
            keys,
            children,
            values,
            // next: None, // TODO
            // next: self.next.take(),
        });

        let new_key = if self.is_leaf {
            new_node.keys[0].clone()
        } else {
            new_node.keys.remove_index(0)
        };

        (new_key, new_node)
    }

    pub fn needs_split(&self, order: usize) -> bool {
        self.keys.len() >= (order * 2)
    }
}

#[derive(Debug)]
pub struct FractalTreeRead<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    root: NodeRead<K, V>,
}

impl<K, V> FractalTreeRead<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + std::fmt::Display,
    V: std::fmt::Debug + Copy,
{
    pub fn search(&self, key: &K) -> Option<V> {
        self.root.search(&key)
    }

    // Todo: add search many function

}

// Conversion impl for FractalTree to FractalTreeRead
impl<K, V> From<FractalTree<K, V>> for FractalTreeRead<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + Default,
    V: std::fmt::Debug + Copy,
{
    fn from(tree: FractalTree<K, V>) -> Self {
        FractalTreeRead {
            root: tree.root.into()
        }
    }
}

#[derive(Debug, Encode, Decode)]
pub struct FractalTreeDisk<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + 'static,
    V: std::fmt::Debug + Copy + 'static,
{
    root: NodeDisk<K, V>,
}

// Conversion impl for FractalTree to FractalTreeRead
impl<K, V> From<FractalTreeRead<K, V>> for FractalTreeDisk<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + Default,
    V: std::fmt::Debug + Copy,
{
    fn from(tree: FractalTreeRead<K, V>) -> Self {
        FractalTreeDisk {
            root: tree.root.into()
        }
    }
}

#[derive(Debug)]
pub struct NodeRead<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub keys: SortedVec<K>,
    pub children: Option<Vec<Box<NodeRead<K, V>>>>,
    pub values: Option<Vec<V>>,
}

// Conversion for Box<Node> to Box<NodeRead>
impl<K, V> From<Box<Node<K, V>>> for Box<NodeRead<K, V>>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + Default,
    V: std::fmt::Debug + Copy,
{
    fn from(node: Box<Node<K, V>>) -> Self {
        Box::new((*node).into())
    }
}

// Conversion for Node to NodeRead
// Todo: conversion may benefit from bumpalo or arch?
impl<K, V> From<Node<K, V>> for NodeRead<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + Default,
    V: std::fmt::Debug + Copy,
{
    fn from(node: Node<K, V>) -> Self {
        let Node {
            is_root,
            is_leaf,
            keys,
            children,
            values,
            buffer: _,
        } = node;

        let children = if children.is_some() {
            let children = children.unwrap();
            let mut new_children = Vec::with_capacity(children.len());
            for child in children {
                new_children.push(child.into());
            }
            Some(new_children)
        } else {
            None
        };

        NodeRead {
            is_root,
            is_leaf,
            keys,
            children,
            values,
        }
    }
}

impl<K, V> NodeRead<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    pub fn search(&self, key: &K) -> Option<V>
    where
        K: PartialOrd + PartialEq + Eq,
    {
        let i = self.keys.binary_search(&key);
       
        if self.is_leaf {
            let i = match i {
                Ok(i) => i,
                Err(_) => return None,
            };
            #[cfg(debug_assertions)]
            assert!(i < self.values.as_ref().unwrap().len());
            Some(self.values.as_ref().unwrap()[i])
        } else {
            let i = match i {
                Ok(i) => i + 1,
                Err(i) => i,
            };

            self.children.as_ref().unwrap()[i].search(key)
        }
    } 
}

#[derive(Debug, Encode, Decode)]
pub struct NodeDisk<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + 'static,
    V: std::fmt::Debug + Copy + 'static,
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub keys: Vec<K>,
    pub children: Option<Vec<Box<NodeDisk<K, V>>>>,
    pub values: Option<Vec<V>>,
}

// Conversion for Box<Node> to Box<NodeRead>
impl<K, V> From<Box<NodeRead<K, V>>> for Box<NodeDisk<K, V>>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + Default,
    V: std::fmt::Debug + Copy,
{
    fn from(node: Box<NodeRead<K, V>>) -> Self {
        Box::new((*node).into())
    }
}

impl<K, V> From<NodeRead<K, V>> for NodeDisk<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + Default,
    V: std::fmt::Debug + Copy,
{
    fn from(node: NodeRead<K, V>) -> Self {
        let NodeRead {
            is_root,
            is_leaf,
            keys,
            children,
            values,
        } = node;

        NodeDisk {
            is_root,
            is_leaf,
            keys: keys.into_vec(),
            children: children.map(|children| {
                children.into_iter().map(|child| child.into()).collect()
            }),
            values,
        }

    }
}

// Modified
// https://www.bazhenov.me/posts/faster-binary-search-in-rust/
pub fn binary_search_branchless<K>(data: &[K], target: &K) -> usize
where
        K: PartialOrd + PartialEq + Ord + Eq + Clone + Copy,
{
    let mut idx = 1;
    while idx < data.len() {
        // TODO: Add (But arch specific)
        // unsafe {
        // let prefetch = data.as_ptr().wrapping_offset(2 * idx as isize);
        // _mm_prefetch::<_MM_HINT_T0>(ptr::addr_of!(prefetch) as *const i8);
        // }
        let el = data[idx];
        idx = 2 * idx + usize::from(el < *target);
        println!("{idx:b}");
    }

    idx >>= idx.trailing_ones() + 1;
    println!("final: {idx:b}");

    idx
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::prelude::*;
    use xxhash_rust::xxh3::xxh3_64;
    use human_size::{Size, SpecificSize, Kilobyte};

    #[test]
    fn split() {
        let mut node = super::Node {
            is_root: false,
            is_leaf: true,
            keys: unsafe { SortedVec::from_sorted(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]) },
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

        let mut node = super::Node {
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
    fn basic_tree() {
        let mut tree = super::FractalTree::new(6, 3);
        tree.insert(0, 0);

        let mut rng = thread_rng();
        let mut values = (0..1024_u64).collect::<Vec<u64>>();
        values.shuffle(&mut rng);

        for i in values.iter() {
            tree.insert(*i, *i);
        }

        assert!(!tree.root.is_leaf);
    }

    #[test]
    fn simple_insertions() {
        let mut tree = super::FractalTree::new(6, 3);
        tree.insert(1, "one");
        tree.insert(2, "two");
        tree.insert(3, "three");
        tree.insert(4, "four");
        tree.insert(5, "five");
        tree.insert(6, "six");
        tree.insert(7, "seven");
        tree.insert(8, "eight");
        tree.insert(9, "nine");
        tree.insert(10, "ten");
    }

    #[test]
    fn tree_structure() {
        let mut tree = super::FractalTree::new(8, 3);

        let mut rng = thread_rng();
        let mut values = (0..1024_u64).collect::<Vec<u64>>();
        values.shuffle(&mut rng);

        for i in values.iter() {
            tree.insert(*i, *i);
        }

        // Iterate through the tree, all leaves should have keys.len() == vales.len()
        // All internal nodes should have keys.len() == children.len() - 1
        let mut stack = vec![&tree.root];
        while let Some(node) = stack.pop() {
            if node.is_leaf {
                assert_eq!(node.keys.len(), node.values.as_ref().unwrap().len());
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
    fn search() {
        let mut rng = thread_rng();
        let mut values_1024 = (0..1024_u64).collect::<Vec<u64>>();
        values_1024.shuffle(&mut rng);

        let mut values_8192 = (0..8192_u64).collect::<Vec<u64>>();
        values_8192.shuffle(&mut rng);

        let mut tree = super::FractalTree::new(8, 8);

        for i in values_1024.iter() {
            tree.insert(*i, *i);
        }

        tree.flush_all();

        for i in values_1024.iter() {
            assert_eq!(tree.search(*i), Some(*i));
        }

        let mut tree = super::FractalTree::new(16, 32);

        for i in values_8192.iter() {
            tree.insert(*i, *i);
        }

        tree.flush_all();

        for i in values_8192.iter() {
            assert_eq!(tree.search(*i), Some(*i));
        }

        // Find value does not exist
        assert_eq!(tree.search(8192), None);

        let mut tree = super::FractalTree::new(64, 32);
        for i in 0..(1024 * 1024) {
            tree.insert(i as u64, i as u64);
        }

        tree.flush_all();

        for i in 0..(1024 * 1024) {
            assert!(tree.search(i as u64) == Some(i as u64), "i: {}", i);
        }

        // Things that should not be found
        assert!(tree.search(1024 * 1024) == None);
        assert!(tree.search(1024 * 1024 + 1) == None);

        // New tree
        let mut tree = super::FractalTree::new(8, 4);
        for i in 1024..2048_u64 {
            tree.insert(i, i);
        }

        tree.flush_all();

        for i in 1024..2048_u64 {
            assert_eq!(tree.search(i), Some(i));
        }

        // Things that should not be found
        for i in 0..1024_u64 {
            assert_eq!(tree.search(i), None);
        }
        for i in 2048..4096_u64 {
            assert_eq!(tree.search(i), None);
        }
    }
    
    /*
    #[test]
    fn eytzinger_tests() {

        let sorted_vec = (15..20).collect::<Vec<u64>>();
        let mut eytzinger_vec = sorted_vec.clone();
        eytzinger_vec.eytzingerize(&mut eytzinger::permutation::InplacePermutator);
        let mut eyt = vec![0];
        eyt.extend(eytzinger_vec.clone());
        let eytzinger_vec = eyt;

        let eyt_index = binary_search_branchless(&eytzinger_vec, &0);
        let eyt_index = binary_search_branchless(&eytzinger_vec, &1000);

        let sorted_vec = (15..20).collect::<Vec<u64>>();
        let mut eytzinger_vec = sorted_vec.clone();
        eytzinger_vec.eytzingerize(&mut eytzinger::permutation::InplacePermutator);
        let mut eyt = vec![0];
        eyt.extend(eytzinger_vec.clone());
        let eytzinger_vec = eyt;

        println!("{:#?}", eytzinger_vec);
        let eyt_index = binary_search_branchless(&eytzinger_vec, &15);
        println!("Index: {}", eyt_index);
        let translated_index = eytzinger::foundation::get_permutation_element(sorted_vec.len()-1, eyt_index-1);
        println!(
            "Sorted Vec Index: {}",
            translated_index
        );
        println!("Sorted Vec Value: {}", sorted_vec[translated_index]);

        // Not found value
        let sorted_vec = (0..1000).step_by(15).collect::<Vec<u64>>();
        let mut eytzinger_vec = sorted_vec.clone();
        eytzinger_vec.eytzingerize(&mut eytzinger::permutation::InplacePermutator);
        let mut eyt = vec![0];
        eyt.extend(eytzinger_vec.clone());
        let eytzinger_vec = eyt;

        println!("{:#?}", eytzinger_vec);
        let eyt_index = binary_search_branchless(&eytzinger_vec, &30);
        let normal_search = sorted_vec.binary_search(&30);
        println!("Normal Search: {:?}", normal_search);
        println!("Index: {:?}", eyt_index);
        let translated_index = eytzinger::foundation::get_permutation_element(sorted_vec.len()-1, eyt_index-1);
        println!(
            "Sorted Vec Index: {}",
            translated_index
        );
        println!("Sorted Vec Value: {}", sorted_vec[translated_index]);

        panic!();
    }
    */

    #[test]
    fn search_noderead() {
        let mut rng = thread_rng();
        let mut values_1024 = (0..1024_u64).collect::<Vec<u64>>();
        values_1024.shuffle(&mut rng);

        let mut values_8192 = (0..8192_u64).collect::<Vec<u64>>();
        values_8192.shuffle(&mut rng);

        let mut tree = super::FractalTree::new(32, 8);

        for i in values_1024.iter() {
            tree.insert(*i, *i);
        }

        tree.flush_all();

        let tree = FractalTreeRead::from(tree);

        for i in values_1024.iter() {
            assert_eq!(tree.search(i), Some(*i));
        }
    }

    #[ignore]
    #[test]
    fn size_of_super_large() {
        let values128m = (0..128_369_206_u64)
        .map(|x| xxh3_64(&x.to_le_bytes()))
        .collect::<Vec<u64>>();

        let mut tree: FractalTree<_, _> = FractalTree::new(1024, 1024);
        for i in values128m.iter() {
            tree.insert(*i, *i);
        }
        tree.flush_all();
        let tree: FractalTreeRead<_, _> = tree.into();
        let tree: FractalTreeDisk<_, _> = tree.into();

        let config = config::standard();
        let encoded: Vec<u8> = bincode::encode_to_vec(&tree, config).unwrap();
        println!("Size of 128m tree: {}", encoded.len());

        let size2: SpecificSize<human_size::Gigabyte> = format!("{} B", encoded.len()).parse().unwrap();

        println!("Size of 128m tree: {}", size2);
        
        panic!();
    }


}
