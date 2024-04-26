#![feature(is_sorted)]
#![feature(trait_alias)]
#![feature(step_trait)]

pub mod build;
pub mod conversion;
pub mod disk;
pub mod fractal;

use std::{iter::Step, ops::{AddAssign, RangeBounds, SubAssign}};

pub use build::*;
pub use conversion::*;
pub use disk::*;
pub use fractal::*;

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
    + Step
    + TryFrom<usize>
    + TryFrom<u64>
    + TryFrom<u32>
    + TryFrom<u16>
    + TryFrom<u8>
    ;

pub trait Value = 'static + std::fmt::Debug + Encode + Decode + Clone;

// Impl can't be a trait because need for buffer for Disk version and
// no need for Read

#[cfg(test)]
mod test
{
    use super::*;

    #[test]
    fn test_duplicate_tree()
    {
        let mut tree: FractalTreeBuild<_, _, false> =
            FractalTreeBuild::new(8, 16);
        for _ in 0..8192 {
            tree.insert(1, 1);
        }
        tree.flush_all();
        let tree: FractalTreeDisk<u32, u32, false> = tree.into();

        assert!(tree.root.children.is_some());
        assert!(tree.root.children.as_ref().unwrap().len() > 0);

        println!(
            "Found {} children",
            tree.root.children.as_ref().unwrap().len()
        );
    }
}
