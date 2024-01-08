//! B+ Tree used for Index as well as SeqLocs?
//! As this is a write-once data structure, we don't need to reorganize anything after (so insertions, etc..).

struct BTree {
    root: Option<Box<Node>>,
    degree: usize,
    // other metadata fields
}

impl BTree {
    fn new(items: Vec) -> Self { /* ... */ }
    fn suggest_degree(total_items: usize) -> usize { 
        let mut degree = 1;
        while degree.pow(2) < total_items {
            degree += 1;
        }
        degree
     }
    fn search(&self, key: u64) -> Option<&Vec<Option<Vec<u64>>>> { /* ... */ }       
}


struct Node {
    keys: Vec<u64>,
    data: Vec<Vec<Option<Vec<u64>>>>, // or a more suitable data type
    children: Vec<Box<Node>>,
    is_leaf: bool,
    // other node-specific fields
}
