//! B+ Tree used for Index as well as SeqLocs?
//! As this is a write-once data structure, we don't need to reorganize anything after (so insertions, etc..).
//! 
//! Todo: Set it up so we can append nodes to it (keys should already be in order, remember?). For very large indices.
//! But this is a TODO for now...

struct BTree<K: KeyType<K: KeyType, V: ValueType>> {
    root: Option<Box<Node>>,
    order: usize,
    depth: usize,
    _value_type: PhantomData<V>,
    _key_type: PhantomData<K>,
}

impl<K: KeyType, V: ValueType> BTree<K, V> {
    fn new(order: usize, items: Vec<(K, V)>) -> Self {
        // Keys should be sorted
        assert!(items.is_sorted_by_key(|(k, _)| k));

        // Create root node
        let root = Node::new(degree);

        let mut tree = Self {
            root: Some(Box::new(root)),
            order,
            depth: 0,
            _value_type: PhantomData,
            _key_type: PhantomData,
        };

        // Calculate depth
        tree.depth = tree.depth(items.len());

        // We can calculate the values that split the keys into the nodes
        let split_values: Vec<K> = tree.split_values(items.len());

        
       
    }

    // Calculate the depth of the tree
    fn depth(&self, n: usize) -> usize {
        // Depth should allow stores of 1024 items in the leaf nodes (so if n = 96,000,000 and o=4, then d=9 to store all items in the leaf nodes,
        // with no more than 1,024 items in each leaf node)

        // Formula is
        // o^d * 1024 <= n

        // So we need to solve for d
        let mut d = 1;
        while (self.order.pow(d) * 1024) <= n {
            d += 1;
        }

        d
    }

    fn search(&self, key: u64) -> Option<&Vec<Option<Vec<u64>>>> { /* ... */
    }
}

// Impl for K as u64
impl<V: ValueType> BTree<u64, V> {
    fn split_values(&self, keys: &[u64]) -> Vec<u64> {
        // Split values are the values that split the keys into the nodes
        // So if we have 96,000,000 keys, and o=4, then we need to find the 3 keys that split into even chunks

        let splits: Vec<u64> = keys
            .chunks(self.order)
            .map(|chunk| chunk[0])
            .collect();

        splits
    }
}

/// B+ Tree Node
pub struct Node<K: KeyType> {
    keys: [K; 3],
    children: [Option<Box<Node>>; 4],
}

impl Node {
    fn new(degree: usize) -> Self {
        Self {
            keys: [0; 3],
            children: [None; degree + 1],
        }
    }
}

pub struct Leaf {
    keys: [u64; 3],
    values: [Option<Vec<u64>>; 3],
}
