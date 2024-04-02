use super::*;
use libcompression::*;
use sorted_vec::SortedVec;

#[derive(Debug)]
pub struct FractalTreeRead<K, V>
where
    K: Key,
    V: Value,
{
    pub root: NodeRead<K, V>,
}

impl<K, V> FractalTreeRead<K, V>
where
    K: Key,
    V: Value,
{
    // Assume root node is already loaded
    pub fn search(&self, key: &K) -> Option<V> {
        self.root.search(&key)
    }

    pub fn len(&self) -> usize {
        self.root.len()
    }

    // Todo: add search many function
}

#[derive(Debug)]
pub struct NodeRead<K, V>
where
    K: Key,
    V: Value,
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub keys: SortedVec<K>,
    pub children: Option<Vec<Box<NodeRead<K, V>>>>,
    pub values: Option<Vec<V>>,
}

impl<K, V> NodeRead<K, V>
where
    K: Key,
    V: Value,
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
                Ok(i) => i.saturating_add(1),
                Err(i) => i,
            };

            self.children.as_ref().unwrap()[i].search(key)
        }
    }

    // Have to do iteratively
    pub fn len(&self) -> usize {
        if self.is_leaf {
            self.keys.len()
        } else {
            let mut len = 0;

            for i in 0..self.children.as_ref().unwrap().len() {
                len += self.children.as_ref().unwrap()[i].len();
            }

            len
        }
    }
}
