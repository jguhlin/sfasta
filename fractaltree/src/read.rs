use super::*;
use libcompression::*;
use sorted_vec::SortedVec;

#[derive(Debug)]
pub struct FractalTreeRead
{
    pub root: NodeRead,
}

impl FractalTreeRead
{
    // Assume root node is already loaded
    pub fn search(&self, key: u64) -> Option<u32>
    {
        self.root.search(key)
    }

    pub fn len(&self) -> usize
    {
        self.root.len()
    }

    // Todo: add search many function
}

#[derive(Debug)]
pub struct NodeRead
{
    pub is_root: bool,
    pub is_leaf: bool,
    pub keys: SortedVec<u64>,
    pub children: Option<Vec<Box<NodeRead>>>,
    pub values: Option<Vec<u32>>,
}

impl NodeRead
{
    pub fn search(&self, key: u64) -> Option<u32>
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
    pub fn len(&self) -> usize
    {
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
