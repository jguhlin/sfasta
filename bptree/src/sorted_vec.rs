// Ideas:
// Bumpalo / pulp to speed things up?
// succinct data structures to compact storage?
//
// Todo:
// Try sorted_vec ?
// Make storable on disk
// Batch insertion
// Able to load only part of the tree from disk
//
// use bumpalo::Bump;
// use pulp::Arch;
use sorted_vec::SortedVec;

use std::marker::PhantomData;

// This is an insertion-only B+ tree, deletions are simply not supported
// Meant for a read-many, write-once on-disk database

#[derive(Debug)]
pub struct SortedVecTree<'tree, K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    root: Node<K, V>,
    order: usize,
    phantom: PhantomData<&'tree Node<K, V>>,
}

impl<'tree, K, V> SortedVecTree<'tree, K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    pub fn new(order: usize) -> Self {
        let mut root = Node::leaf(order);
        root.is_root = true;
        SortedVecTree {
            root,
            order,
            phantom: PhantomData,
        }
    }

    pub fn insert(&mut self, key: K, value: V)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug + Copy,
    {
        assert!(self.root.keys.is_sorted());
        match self.root.insert(self.order, key, value) {
            InsertionAction::Success => (),
            InsertionAction::NodeSplit(new_key, mut new_node) => {
                let old_root = Node::internal(self.order);
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
    }

    pub fn search(&self, key: K) -> Option<V>
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug + Copy,
    {
        self.root.search(key)
    }

    // TODO: Specialization for when key is u64
    pub fn batch_insert(&mut self, keys: Vec<K>, values: Vec<V>)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy + Default,
        V: std::fmt::Debug + Copy,
    {
        unimplemented!(); // TODO: Need to do more, easy to make leaf nodes, but making internal nodes and root nodes needs more thinking
        assert!(keys.len() == values.len());

        // Sort the keys and values together
        let mut zipped = keys.iter().zip(values.iter()).collect::<Vec<_>>();
        zipped.sort_by(|&(&k1, _), &(k2, _)| k1.cmp(k2));
        // let (keys, values): (Vec<&K>, Vec<&V>) = zipped.into_iter().unzip();

        let mut nodes = Vec::new();

        let chunks = zipped.chunks(self.order);
        for chunk in chunks {
            let mut keys = SortedVec::with_capacity(self.order);
            let mut values = Vec::with_capacity(self.order);
            for (key, value) in chunk {
                keys.push(**key);
                values.push(**value);
            }
            let node = Node {
                is_root: false,
                is_leaf: true,
                keys,
                children: None,
                values: Some(values),
                next: None,
            };
            nodes.push(node);
        }
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
    pub next: Option<Box<Node<K, V>>>, // Does this need to be a u64 until loaded?
}

impl<K, V> Node<K, V>
where
    K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
    V: std::fmt::Debug + Copy,
{
    pub fn internal(order: usize) -> Self {
        Node {
            is_root: false,
            is_leaf: false,
            keys: SortedVec::with_capacity(order),
            children: Some(Vec::with_capacity(order)),
            values: None,
            next: None,
        }
    }

    pub fn leaf(order: usize) -> Self {
        Node {
            is_root: false,
            is_leaf: true,
            keys: SortedVec::with_capacity(order),
            children: None,
            values: Some(Vec::with_capacity(order)),
            next: None,
        }
    }

    pub fn search(&self, key: K) -> Option<V>
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug + Copy,
    {
        let i = self.keys.binary_search(&key);

        if self.is_leaf {
            let i = match i {
                Ok(i) => i,
                Err(_) => return None, // This is the leaf, if it's not found here it won't be found
            };
            // Leaf K->V are 1 to 1 mapping
            assert!(i < self.values.as_ref().unwrap().len());
            Some(self.values.as_ref().unwrap()[i])
        } else {
            // B+ tree search, so we need to find the correct child node
            let i = match i {
                Ok(i) => i + 1,
                Err(i) => i,
            };

            self.children.as_ref().unwrap()[i].search(key)
        }
    }

    pub fn insert(&mut self, order: usize, key: K, value: V) -> InsertionAction<K, V>
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        if self.is_leaf {
            let i = self.keys.insert(key);
            self.values.as_mut().unwrap().insert(i, value);
        } else {
            let i = match self.keys.binary_search(&key) {
                Ok(i) => i,
                Err(i) => i,
            };
            // Insert into child node
            match self.children.as_mut().unwrap()[i].insert(order, key, value) {
                InsertionAction::NodeSplit(new_key, new_node) => {
                    let i = self.keys.insert(new_key) + 1;

                    if i >= self.children.as_ref().unwrap().len() {
                        self.children.as_mut().unwrap().push(new_node);
                    } else {
                        self.children
                            .as_mut()
                            .unwrap()
                            .insert(i, new_node);
                    }
                }
                InsertionAction::Success => (),
            };
        }

        if self.needs_split(order) {
            let (new_key, new_node) = self.split();
            InsertionAction::NodeSplit(new_key, new_node)
        } else {
            InsertionAction::Success
        }
    }

    pub fn split(&mut self) -> (K, Box<Node<K, V>>)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        assert!(self.keys.is_sorted());
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

        assert!(mid < self.keys.len());
        let keys = self.keys.split_at(mid);
        let orig_keys = unsafe { SortedVec::from_sorted(keys.0.to_vec()) };
        let keys = unsafe { SortedVec::from_sorted(keys.1.to_vec()) };
        self.keys = orig_keys;       

        let mut new_node = Box::new(Node {
            is_root: false,
            is_leaf: self.is_leaf,
            keys,
            children,
            values,
            next: None, // TODO
                        // next: self.next.take(),
        });

        let new_key = if self.is_leaf {
            new_node.keys[0].clone()
        } else {
            new_node.keys.remove_index(0)
        };

        println!("New key: {:?}", new_key);

        (new_key, new_node)
    }

    pub fn needs_split(&self, order: usize) -> bool {
        self.keys.len() >= (order * 2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split() {
        let mut node = super::Node {
            is_root: false,
            is_leaf: true,
            keys: unsafe { SortedVec::from_sorted(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]) },
            children: None,
            values: Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
            next: None,
        };

        // Initial node: keys: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        // After split:
        // Node: keys: [1, 2, 3, 4, 5], values: [1, 2, 3, 4, 5]
        // New Node: keys: [6, 7, 8, 9, 10, 11], values: [6, 7, 8, 9, 10, 11]
        // New key: 6

        let (new_key, new_node) = node.split();
        assert_eq!(new_key, 6);
        assert_eq!(new_node.keys, unsafe { SortedVec::from_sorted(vec![6, 7, 8, 9, 10, 11]) });
        assert_eq!(new_node.values, Some(vec![6, 7, 8, 9, 10, 11]));
        assert_eq!(node.keys, unsafe { SortedVec::from_sorted(vec![1, 2, 3, 4, 5]) });
        assert_eq!(node.values, Some(vec![1, 2, 3, 4, 5]));

        let mut node = super::Node {
            is_root: false,
            is_leaf: true,
            keys: SortedVec::from_unsorted((0..28).collect()),
            children: None,
            values: Some((0..28).collect()),
            next: None,
        };

        let (new_key, new_node) = node.split();
        assert!(new_key == 14);
        assert_eq!(new_node.keys, SortedVec::from_unsorted((14..28).collect::<Vec<_>>()));
        assert_eq!(new_node.values, Some((14..28).collect::<Vec<_>>()));
        assert_eq!(node.keys.to_vec(), (0..14).collect::<Vec<_>>());
        assert_eq!(node.values, Some((0..14).collect::<Vec<_>>()));

        let (new_key, new_node_) = node.split();
        assert!(new_key == 7);
        assert_eq!(new_node_.keys.to_vec(), (7..14).collect::<Vec<_>>());
        assert_eq!(new_node_.values, Some((7..14).collect::<Vec<_>>()));
        assert_eq!(node.keys.to_vec(), (0..7).collect::<Vec<_>>());
        assert_eq!(node.values, Some((0..7).collect::<Vec<_>>()));
    }

    #[test]
    fn basic_tree() {
        let mut tree = super::SortedVecTree::new(6);
        tree.insert(0, 0);

        for i in 1..=7 {
            tree.insert(i, i);
        }

        for i in 8..18 {
            tree.insert(i, i);
        }
        assert!(!tree.root.is_leaf);
    }

    #[test]
    fn simple_insertions() {
        let mut tree = super::SortedVecTree::new(6);
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
        let mut tree = super::SortedVecTree::new(8);
        for i in 0..1024_u64 {
            tree.insert(i, i);
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
        let mut tree = super::SortedVecTree::new(8);
        for i in 0..1024_u64 {
            tree.insert(i, i);
        }

        // When order is 8, 64 can't be found
        // Let's do the search "manually"

        for i in 0..1024_u64 {
            assert_eq!(tree.search(i), Some(i));
        }

        let mut tree = super::SortedVecTree::new(96);
        for i in 0..8192_u64 {
            tree.insert(i, i);
        }

        for i in 0..8192_u64 {
            assert_eq!(tree.search(i), Some(i));
        }

        // Find value does not exist
        assert_eq!(tree.search(8192), None);

        let mut tree = super::SortedVecTree::new(64);
        for i in 0..(1024 * 1024) {
            tree.insert(i as u64, i as u64);
        }
        for i in 0..(1024 * 1024) {
            assert!(tree.search(i as u64) == Some(i as u64), "i: {}", i);
        }

        // Things that should not be found
        assert!(tree.search(1024 * 1024) == None);
        assert!(tree.search(1024 * 1024 + 1) == None);

        // New tree
        let mut tree = super::SortedVecTree::new(8);
        for i in 1024..2048_u64 {
            tree.insert(i, i);
        }

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
}
