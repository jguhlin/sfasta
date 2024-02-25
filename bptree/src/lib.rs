use pulp::Arch;
use std::marker::PhantomData;

#[derive(Debug)]
pub struct BPlusTree<'tree, K, V> {
    root: Node<K, V>,
    order: u8,
    phantom: PhantomData<&'tree Node<K, V>>,
}

impl<'tree, K, V> BPlusTree<'tree, K, V> {
    pub fn new(order: u8) -> Self {
        BPlusTree {
            root: Node::leaf(),
            order,
            phantom: PhantomData,
        }
    }

    pub fn insert(&mut self, key: K, value: V)
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        match self.root.insert(self.order as usize, key, value) {
            InsertionAction::Success => (),
            InsertionAction::NodeSplit(new_key, new_node) => {
                let mut old_root = Node::internal();
                std::mem::swap(&mut self.root, &mut old_root);

                if old_root.keys[0] < new_key {
                    self.root.keys.push(new_key);
                    self.root.children.as_mut().unwrap().push(Box::new(old_root));
                    self.root.children.as_mut().unwrap().push(new_node);
                    
                } else {
                    self.root.keys.push(old_root.keys[0]);
                    self.root.children.as_mut().unwrap().push(new_node);
                    self.root.children.as_mut().unwrap().push(Box::new(old_root));
                }
            },
        }
    }
}

// Must use
#[must_use]
pub enum InsertionAction<K, V> {
    Success,
    NodeSplit(K, Box<Node<K, V>>),
}

#[derive(Debug)]
pub struct Node<K, V> {
    pub is_leaf: bool,
    pub keys: Vec<K>,
    pub children: Option<Vec<Box<Node<K, V>>>>,
    pub values: Option<Vec<V>>,
    pub next: Option<Box<Node<K, V>>>, // Does this need to be a u64 until loaded?
}

impl<K, V> Node<K, V> {
    pub fn internal() -> Self {
        Node {
            is_leaf: false,
            keys: Vec::new(),
            children: Some(Vec::new()),
            values: None,
            next: None,
        }
    }

    pub fn leaf() -> Self {
        Node {
            is_leaf: true,
            keys: Vec::new(),
            children: None,
            values: Some(Vec::new()),
            next: None,
        }
    }

    pub fn insert(&mut self, order: usize, key: K, value: V) -> InsertionAction<K, V>
    where
        K: PartialOrd + PartialEq + Ord + Eq + std::fmt::Debug + Clone + Copy,
        V: std::fmt::Debug,
    {
        let i = self
            .keys
            .iter()
            .map(|k| *k > key)
            .position(|x| x)
            .unwrap_or(self.keys.len());

        if self.is_leaf {
            // Find insertion point
            self.keys.insert(i, key);
            self.values.as_mut().unwrap().insert(i, value);
            assert!(self.keys.len() == self.values.as_ref().unwrap().len());
        } else {
            // Insert into child node
            if self.children.as_mut().unwrap().len() <= i {
                self.children.as_mut().unwrap().push(Box::new(Node::leaf()));
            }
            match self.children.as_mut().unwrap()[i].insert(order, key, value) {
                InsertionAction::NodeSplit(new_key, new_node) => {
                    // TODO: Make assertion that new_key is in the correct place
                    assert!(
                        i == self
                            .keys
                            .iter()
                            .map(|k| *k > new_key)
                            .position(|x| x)
                            .unwrap_or(self.keys.len())
                    );

                    // We don't insert keys at the start or the end
                    self.keys.insert(i, new_key);
                    self.children.as_mut().unwrap().insert(i + 1, new_node);

                    if self.keys.len() == self.children.as_ref().unwrap().len() {
                        // Rekey
                        let mut new_keys = Vec::new();
                        for child in self.children.as_ref().unwrap().iter() {
                            new_keys.push(child.keys[0]);
                        }
                        // This is a B+ tree, drop keys
                        new_keys.pop();
                        self.keys = new_keys;

                    }
                    assert!(self.keys.len() == self.children.as_ref().unwrap().len() - 1);
                },
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
    {
        let mid = self.keys.len() / 2;

        let values = if self.values.is_some() {
            Some(self.values.as_mut().unwrap().split_off(mid))
        } else {
            None
        };

        let children = if self.children.is_some() {
            Some(self.children.as_mut().unwrap().split_off(mid))
        } else {
            None
        };

        let new_node = Box::new(Node {
            is_leaf: self.is_leaf,
            keys: self.keys.split_off(mid),
            children,
            values,
            next: self.next.take(),
        });
        (new_node.keys[0], new_node)
    }

    pub fn needs_split(&self, order: usize) -> bool {
        self.keys.len() >= 2 * order
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn simple_insertions() {
        let mut tree = super::BPlusTree::new(3);
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
        let mut tree = super::BPlusTree::new(2);
        tree.insert(1 as u64, 1 as u64);
        tree.insert(15 as u64, 1 as u64);
        tree.insert(0 as u64, 1 as u64);
        tree.insert(32 as u64, 1 as u64);
        tree.insert(128 as u64, 1 as u64);
        tree.insert(64 as u64, 1 as u64);
        tree.insert(31 as u64, 1 as u64);
        tree.insert(97 as u64, 1 as u64);
        tree.insert(95 as u64, 1 as u64);
        tree.insert(96 as u64, 1 as u64);
        tree.insert(1005 as u64, 1 as u64);
        tree.insert(2 as u64, 1 as u64);
        tree.insert(3 as u64, 1 as u64);
        tree.insert(4 as u64, 1 as u64);
        tree.insert(5 as u64, 1 as u64);

        // Iterate through the tree, all leaves should have keys.len() == vales.len()
        // All internal nodes should have keys.len() == children.len() - 1
        let mut stack = vec![&tree.root];
        while let Some(node) = stack.pop() {
            if node.is_leaf {
                println!("{:#?}", node);
                assert_eq!(node.keys.len(), node.values.as_ref().unwrap().len());
            } else {
                println!("{:#?}", node);
                assert_eq!(node.keys.len(), node.children.as_ref().unwrap().len() - 1);
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
}
