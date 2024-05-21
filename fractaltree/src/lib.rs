#![feature(async_iterator)]
#![feature(is_sorted)]
#![feature(trait_alias)]

pub mod build;
pub mod conversion;
pub mod fractal;

pub mod disk;

#[cfg(feature = "async")]
pub mod disk_async;

#[cfg(feature = "async")]
pub use disk_async::*;

pub use build::*;
pub use disk::*;

use std::ops::{AddAssign, SubAssign};

use bincode::{Decode, Encode};

pub trait Key = 'static
    + PartialOrd
    + PartialEq
    + Ord
    + Eq
    + std::fmt::Debug
    + Clone
    + Default
    + Encode
    + Decode
    + Default
    + num::traits::Unsigned
    + Copy
    + SubAssign
    + AddAssign
    + Send
    + Sync
    + Default;

pub trait Value =
    'static + std::fmt::Debug + Encode + Decode + Clone + Send + Sync + Default;

#[cfg(test)]
mod test
{
    use super::*;

    #[test]
    fn test_duplicate_tree()
    {
        let mut tree = FractalTreeBuild::new(8, 16);
        for _ in 0..8192 {
            tree.insert(1, 1);
        }
        tree.flush_all();
        let tree: FractalTreeDisk<u32, u32> = tree.into();

        assert!(tree.root.children.is_some());
        assert!(tree.root.children.as_ref().unwrap().len() > 0);

        println!(
            "Found {} children",
            tree.root.children.as_ref().unwrap().len()
        );
    }
}
