// Ideas:
// Bumpalo / pulp to speed things up?
// succinct data structures to compact storage?
//
// Todo:
// Trying out fractal tree
//
// use bumpalo::Bump;
// use pulp::Arch;

use std::marker::PhantomData;

// This is an insertion-only B+ tree, deletions are simply not supported
// Meant for a read-many, write-once on-disk database

#[derive(Debug)]
pub struct FractalTree<'tree, K, V> {
    root: Node<K, V>,
    order: usize,
    buffer_size: usize,
    phantom: PhantomData<&'tree Node<K, V>>,
}

impl<'tree, K, V> FractalTree<'tree, K, V> {
    pub fn new(order: usize, buffer_size: usize) -> Self {
        let mut root = Node::leaf(order, buffer_size);
        root.is_root = true;
        FractalTree {
            root,
            order,
            phantom: PhantomData,
            buffer_size,
        }
    }

    pub fn insert(&mut self, key: K, value: V)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug + Copy,
    {
        assert!(self.root.keys.is_sorted());
        match self.root.insert(self.order, self.buffer_size, key, value) {
            InsertionAction::Success => (),
            InsertionAction::Buffering => (),
            InsertionAction::NodeSplit(new_key, mut new_node) => {
                let mut old_root = Node::internal(self.order, self.buffer_size);
                old_root.is_root = true;
                std::mem::swap(&mut self.root, &mut old_root);
                let mut old_root = Box::new(old_root);
                old_root.is_root = false;
                new_node.is_root = false;

                if old_root.keys[0] < new_key {
                    self.root.keys.push(new_key);
                    self.root.children.as_mut().unwrap().push(old_root);
                    self.root.children.as_mut().unwrap().push(new_node);
                } else {
                    self.root.keys.push(old_root.keys[0]);
                    self.root.children.as_mut().unwrap().push(new_node);
                    self.root.children.as_mut().unwrap().push(old_root);
                }
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
}

// Must use
#[must_use]
pub enum InsertionAction<K, V> {
    Success,
    NodeSplit(K, Box<Node<K, V>>),
    Buffering,
}

#[derive(Debug)]
pub struct Node<K, V> {
    pub is_root: bool,
    pub is_leaf: bool,
    pub keys: Vec<K>,
    pub children: Option<Vec<Box<Node<K, V>>>>,
    pub values: Option<Vec<V>>,
    pub buffer: Vec<(K, V)>, // This is an insertion-only implementation, otherwise would use an Enum to specify options...
                             // TODO: At the end, this should be only in the "creation" phase, and another datastructure should handle in-memory / loading / etc...
}

impl<K, V> Node<K, V> {
    pub fn internal(order: usize, buffer_size: usize) -> Self {
        Node {
            is_root: false,
            is_leaf: false,
            keys: Vec::with_capacity(order - 1),
            children: Some(Vec::with_capacity(order)),
            values: None,
            buffer: Vec::with_capacity(buffer_size),
        }
    }

    pub fn leaf(order: usize, buffer_size: usize) -> Self {
        Node {
            is_root: false,
            is_leaf: true,
            keys: Vec::with_capacity(order),
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
        if self.is_leaf {
            let i = match self.keys.binary_search(&key) {
                Ok(i) => i,
                Err(_) => return None, // This is the leaf, if it's not found here it won't be found
            };
            // Leaf K->V are 1 to 1 mapping
            assert!(i < self.values.as_ref().unwrap().len());
            Some(self.values.as_ref().unwrap()[i])
        } else {
            // B+ tree search, so we need to find the correct child node
            let i = match self.keys.binary_search(&key) {
                Ok(i) => i + 1,
                Err(i) => i,
            };

            self.children.as_ref().unwrap()[i].search(key)
        }
    }

    pub fn insert(
        &mut self,
        order: usize,
        buffer_size: usize,
        key: K,
        value: V,
    ) -> InsertionAction<K, V>
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        assert!(self.keys.is_sorted());

        self.buffer.push((key, value));
        if self.buffer.len() < buffer_size {
            return InsertionAction::Buffering;
        } else {
            self.flush(order, buffer_size)
        }
    }

    pub fn flush(&mut self, order: usize, buffer_size: usize) -> InsertionAction<K, V>
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        // Sort the buffer
        self.buffer.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        // Insert into the node
        for (key, value) in self.buffer.drain(..) {
            let i = match self.keys.binary_search(&key) {
                Ok(i) => i,
                Err(i) => i,
            };

            if self.is_leaf {
                self.keys.insert(i, key);
                self.values.as_mut().unwrap().insert(i, value);
                assert!(self.keys.len() == self.values.as_ref().unwrap().len());
            } else {
                // Insert into child node
                match self.children.as_mut().unwrap()[i].insert(order, buffer_size, key, value) {
                    InsertionAction::NodeSplit(new_key, new_node) => {
                        let new_node_insertion = match self.keys.binary_search(&new_key) {
                            Ok(i) => i,
                            Err(i) => i + 1,
                        };

                        if new_node_insertion >= self.children.as_ref().unwrap().len() {
                            self.keys.push(new_key);
                            self.children.as_mut().unwrap().push(new_node);
                        } else {
                            self.keys.insert(new_node_insertion, new_key);
                            self.children
                                .as_mut()
                                .unwrap()
                                .insert(new_node_insertion, new_node);
                        }

                        assert!(
                            self.keys.len() == self.children.as_ref().unwrap().len() - 1,
                            "keys: {:#?}, children: {:#?}",
                            self.keys,
                            self.children.as_ref().unwrap()
                        );
                    }
                    InsertionAction::Success => {
                        // Don't have to do anything...
                        assert!(
                            self.keys.len() == self.children.as_ref().unwrap().len() - 1,
                            "keys: {:#?}, children: {:#?}",
                            self.keys,
                            self.children.as_ref().unwrap()
                        );
                    }
                    InsertionAction::Buffering => (),
                };
            }
        }

        assert!(self.keys.is_sorted());

        if self.needs_split(order) {
            let (new_key, new_node) = self.split();

            if self.is_leaf {
                assert!(self.keys.len() == self.values.as_ref().unwrap().len());
            } else {
                assert!(self.keys.len() == self.children.as_ref().unwrap().len() - 1);
            }
            InsertionAction::NodeSplit(new_key, new_node)
        } else {
            if self.is_leaf {
                assert!(self.keys.len() == self.values.as_ref().unwrap().len());
            } else {
                assert!(self.keys.len() == self.children.as_ref().unwrap().len() - 1);
            }
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
            assert!(mid < self.values.as_ref().unwrap().len());
            Some(self.values.as_mut().unwrap().split_off(mid))
        } else {
            None
        };

        let children = if self.children.is_some() {
            assert!(mid < self.children.as_ref().unwrap().len());
            let children = self.children.as_mut().unwrap().split_off(mid + 1);
            assert!(
                children.len() > 1,
                "Split off: {}, Node: {:#?}, Children: {:#?}",
                mid,
                self,
                self.children.as_ref().unwrap()
            );
            Some(children)
        } else {
            None
        };

        assert!(mid < self.keys.len());
        let keys = self.keys.split_off(mid);

        let mut new_node = Box::new(Node {
            is_root: false,
            is_leaf: self.is_leaf,
            keys,
            children,
            values,
            buffer: Vec::with_capacity(self.buffer.capacity()),
        });

        if self.is_leaf {
            assert!(self.keys.len() == self.values.as_ref().unwrap().len());
            assert!(new_node.keys.len() == new_node.values.as_ref().unwrap().len());
        } else {
            assert!(
                self.keys.len() == self.children.as_ref().unwrap().len() - 1,
                "keys: {:#?}, children: {:#?}\n New Node: {:#?}",
                self.keys,
                self.children.as_ref().unwrap(),
                new_node
            );
        }

        assert!(self.keys.is_sorted());
        assert!(new_node.keys.is_sorted());

        if self.is_leaf {
            assert!(self.keys.len() == self.values.as_ref().unwrap().len());
            assert!(new_node.keys.len() == new_node.values.as_ref().unwrap().len());
        } else {
            assert!(
                self.keys.len() == self.children.as_ref().unwrap().len() - 1,
                "{} {}",
                self.keys.len(),
                self.children.as_ref().unwrap().len()
            );
            assert!(
                new_node.keys.len() == new_node.children.as_ref().unwrap().len(),
                "{} {} {:#?}",
                new_node.keys.len(),
                new_node.children.as_ref().unwrap().len(),
                new_node
            );
        }

        let new_key = if self.is_leaf {
            new_node.keys[0].clone()
        } else {
            new_node.keys.remove(0)
        };

        (new_key, new_node)
    }

    // Split root into 2
    pub fn split_root(&mut self) -> (K, Box<Node<K, V>>)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        assert!(self.keys.is_sorted());
        let mid = self.keys.len() / 2;

        let values = if self.values.is_some() {
            assert!(mid < self.values.as_ref().unwrap().len());
            Some(self.values.as_mut().unwrap().split_off(mid))
        } else {
            None
        };

        let children = if self.children.is_some() {
            assert!(mid < self.children.as_ref().unwrap().len());
            let children = self.children.as_mut().unwrap().split_off(mid + 1);
            assert!(
                children.len() > 1,
                "Split off: {}, Node: {:#?}, Children: {:#?}",
                mid,
                self,
                self.children.as_ref().unwrap()
            );
            Some(children)
        } else {
            None
        };

        assert!(mid < self.keys.len());
        let keys = self.keys.split_off(mid);

        let mut new_node = Box::new(Node {
            is_root: false,
            is_leaf: self.is_leaf,
            keys,
            children,
            values,
            buffer: Vec::with_capacity(self.buffer.capacity()),
        });

        if self.is_leaf {
            assert!(self.keys.len() == self.values.as_ref().unwrap().len());
            assert!(new_node.keys.len() == new_node.values.as_ref().unwrap().len());
        } else {
            assert!(
                self.keys.len() == self.children.as_ref().unwrap().len() - 1,
                "keys: {:#?}, children: {:#?}\n New Node: {:#?}",
                self.keys,
                self.children.as_ref().unwrap(),
                new_node
            );
        }

        assert!(self.keys.is_sorted());
        assert!(new_node.keys.is_sorted());

        if self.is_leaf {
            assert!(self.keys.len() == self.values.as_ref().unwrap().len());
            assert!(new_node.keys.len() == new_node.values.as_ref().unwrap().len());
        } else {
            assert!(
                self.keys.len() == self.children.as_ref().unwrap().len() - 1,
                "{} {}",
                self.keys.len(),
                self.children.as_ref().unwrap().len()
            );
            assert!(
                new_node.keys.len() == new_node.children.as_ref().unwrap().len(),
                "{} {} {:#?}",
                new_node.keys.len(),
                new_node.children.as_ref().unwrap().len(),
                new_node
            );
        }

        let new_key = if self.is_leaf {
            new_node.keys[0].clone()
        } else {
            new_node.keys.pop().unwrap()
        };

        (new_key, new_node)
    }

    pub fn needs_split(&self, order: usize) -> bool {
        self.keys.len() >= (order * 2)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn split() {
        let mut node = super::Node {
            is_root: false,
            is_leaf: true,
            keys: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
            children: None,
            values: Some(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]),
            buffer: Vec::with_capacity(256),
        };

        // Initial node: keys: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11]
        // After split:
        // Node: keys: [1, 2, 3, 4, 5], values: [1, 2, 3, 4, 5]
        // New Node: keys: [6, 7, 8, 9, 10, 11], values: [6, 7, 8, 9, 10, 11]
        // New key: 6

        let (new_key, new_node) = node.split();
        assert_eq!(new_key, 6);
        assert_eq!(new_node.keys, vec![6, 7, 8, 9, 10, 11]);
        assert_eq!(new_node.values, Some(vec![6, 7, 8, 9, 10, 11]));
        assert_eq!(node.keys, vec![1, 2, 3, 4, 5]);
        assert_eq!(node.values, Some(vec![1, 2, 3, 4, 5]));

        let mut node = super::Node {
            is_root: false,
            is_leaf: true,
            keys: (0..28).collect(),
            children: None,
            values: Some((0..28).collect()),
            buffer: Vec::with_capacity(256),
        };

        let (new_key, new_node) = node.split();
        assert!(new_key == 14);
        assert_eq!(new_node.keys, (14..28).collect::<Vec<_>>());
        assert_eq!(new_node.values, Some((14..28).collect::<Vec<_>>()));
        assert_eq!(node.keys, (0..14).collect::<Vec<_>>());
        assert_eq!(node.values, Some((0..14).collect::<Vec<_>>()));

        let (new_key, new_node_) = node.split();
        assert!(new_key == 7);
        assert_eq!(new_node_.keys, (7..14).collect::<Vec<_>>());
        assert_eq!(new_node_.values, Some((7..14).collect::<Vec<_>>()));
        assert_eq!(node.keys, (0..7).collect::<Vec<_>>());
        assert_eq!(node.values, Some((0..7).collect::<Vec<_>>()));
    }

    #[test]
    fn basic_tree() {
        let mut tree = super::FractalTree::new(6, 3);
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
        let mut tree = super::FractalTree::new(8, 4);
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
        let mut tree = super::FractalTree::new(8, 4);
        for i in 0..1024_u64 {
            tree.insert(i, i);
        }

        // When order is 8, 64 can't be found
        // Let's do the search "manually"

        for i in 0..1024_u64 {
            assert_eq!(tree.search(i), Some(i));
        }

        let mut tree = super::FractalTree::new(96, 24);
        for i in 0..8192_u64 {
            tree.insert(i, i);
        }

        for i in 0..8192_u64 {
            assert_eq!(tree.search(i), Some(i));
        }

        // Find value does not exist
        assert_eq!(tree.search(8192), None);

        let mut tree = super::FractalTree::new(64, 32);
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
        let mut tree = super::FractalTree::new(8, 4);
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