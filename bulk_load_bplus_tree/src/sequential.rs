use super::*;
use libcompression::*;


#[derive(Debug)]
pub struct BPlusTreeBuild<K, V>
{
    pub entries: Vec<(K, V)>,
    pub order: usize,
}

impl<K: Key, V: Value> BPlusTreeBuild<K, V>
{
    pub fn new(order: usize) -> Self
    {
        Self {
            entries: Vec::new(),
            order,
        }
    }

    pub fn insert(&mut self, key: K, value: V)
    {
        self.entries.push((key, value));
    }

    pub fn count_all_nodes(&self) -> usize
    {
        self.entries.len()
    }

    pub fn reserve(&mut self, additional: usize)
    {
        self.entries.reserve(additional);
    }
}

/// A B+ tree for when keys are 0..len(values)
/// and all values are u64
///
/// Specialized for SFASTA use case, but should be portable eventually
#[derive(Debug)]
pub struct SequentialBPlusTreeBuildU64
{
    // Entries are put in sequentially, thus this is already sorted
    // Keys are 0..len(values)
    pub values: Vec<u64>,
    pub order: usize,
}

impl SequentialBPlusTreeBuildU64
{
    pub fn new(order: usize) -> Self
    {
        Self {
            values: Vec::new(),
            order,
        }
    }

    pub fn insert(&mut self, value: u64)
    {
        self.values.push(value);
    }

    pub fn count_all_nodes(&self) -> usize
    {
        self.values.len()
    }

    pub fn reserve(&mut self, additional: usize)
    {
        self.values.reserve(additional);
    }

    pub fn extend(&mut self, values: &[u64])
    {
        // Reserve space for the new values
        self.values.reserve(values.len());
        self.values.extend_from_slice(values);
    }

    pub fn convert(self) -> SequentialBPlusTreeU64
    {
        // Deconstruct
        let SequentialBPlusTreeBuildU64 { mut values, order } = self;

        if values.is_empty() {
            panic!("Cannot convert an empty B+ tree");
        }

        let mut leaf_nodes = Vec::new();

        let mut i = 0;
        loop {
            if values.len() > order {
                let tail = values.split_off(order);
                let head = std::mem::replace(&mut values, tail);
                let first_key = (i * order) as u64;
                let node = SequentialBPlusTreeU64Node::leaf(head, first_key);
                leaf_nodes.push(node);

                i = i + 1;
            } else {
                let first_key = (i * order) as u64;
                let mut node =
                    SequentialBPlusTreeU64Node::leaf(values, first_key);
                node.is_root = false; // Mark as not root
                leaf_nodes.push(node);
                break;
            }
        }

        // If there's only one leaf node, make it the root and return the tree
        if leaf_nodes.len() == 1 {
            let mut root = leaf_nodes.pop().unwrap();
            root.is_root = true; // Mark as root
            return SequentialBPlusTreeU64 {
                order: self.order,
                start: 0,
                compression: None,
                root,
            };
        }

        // We can't do the fancy loop until the leaves are created, don't
        // forget and try to collapse into a single loop again...

        // Create internal nodes
        let mut current_level_nodes = leaf_nodes;
        let mut new_nodes =
            Vec::with_capacity(current_level_nodes.len() / order);

        while current_level_nodes.len() > 1 {
            loop {
                let node = if current_level_nodes.len() > order {
                    let tail = current_level_nodes.split_off(order);
                    let head =
                        std::mem::replace(&mut current_level_nodes, tail);
                    let head = head.into_iter().map(|v| Box::new(v)).collect();
                    SequentialBPlusTreeU64Node::internal(head)
                } else {
                    let current_level_nodes = current_level_nodes
                        .drain(..)
                        .map(|v| Box::new(v))
                        .collect();
                    SequentialBPlusTreeU64Node::internal(current_level_nodes);
                    break;
                };
                new_nodes.push(node);
            }

            current_level_nodes = new_nodes;
            new_nodes = Vec::with_capacity(current_level_nodes.len() / order);
        }

        assert!(
            current_level_nodes.len() == 1,
            "There should be exactly one root node at the end of the process"
        );
        let mut root = current_level_nodes.pop().unwrap();
        root.is_root = true; // Mark as root

        root.is_root = true; // Mark as root
        SequentialBPlusTreeU64 {
            order: self.order,
            start: 0,
            compression: None,
            root,
        }
    }
}


pub struct SequentialBPlusTreeU64
{
    pub order: usize,

    // This is the on disk location of the root node
    pub start: u64,

    pub compression: Option<CompressionConfig>,
    pub root: SequentialBPlusTreeU64Node,
}

pub struct SequentialBPlusTreeU64Node
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub state: NodeState,
    pub first_key: u64,
    pub children: Option<Vec<Box<SequentialBPlusTreeU64Node>>>,
    pub values: Option<Vec<u64>>,
}

// todo impl encode, decode, borrowdecode
impl SequentialBPlusTreeU64Node
{
    pub fn from_loc(loc: u32) -> Self
    {
        SequentialBPlusTreeU64Node {
            // do not store
            is_root: false,

            // store
            is_leaf: false,

            // do not store
            state: NodeState::OnDisk(loc),
            first_key: 0,

            // store
            children: None,
            values: None,
        }
    }

    pub fn internal(children: Vec<Box<SequentialBPlusTreeU64Node>>) -> Self
    {
        // It's usually computed, and likely this fn is for conversion, so we
        // don't use it But for completeness and for tests after build
        // before disk store...
        let first_key = children[0].first_key;

        SequentialBPlusTreeU64Node {
            is_root: false,
            is_leaf: false,
            state: NodeState::InMemory,
            first_key,
            children: Some(children),
            values: None,
        }
    }

    pub fn leaf(values: Vec<u64>, first_key: u64) -> Self
    {
        SequentialBPlusTreeU64Node {
            is_root: false,
            is_leaf: true,
            state: NodeState::InMemory,
            first_key,
            children: None,
            values: Some(values.to_owned()),
        }
    }

    // todo load fn
    // todo load all fn (loads all children)
    // todo store_dummy (??)
    // todo store (store to disk)
    // todo search
    // todo children_stored_on_disk
}
