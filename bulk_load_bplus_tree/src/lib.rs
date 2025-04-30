#![feature(trait_alias)]

pub mod build;
pub mod conversion;
pub mod disk;

// Going in on specialization, just split it up by that then
pub mod sequential;

use std::ops::{AddAssign, SubAssign};

#[derive(Debug, Clone)]
pub enum NodeState
{
    InMemory,
    Compressed(Vec<u8>),
    OnDisk(u32),
}

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
    + Default
    + num::traits::Unsigned
    + Copy
    + SubAssign
    + AddAssign;

pub trait Value = 'static + std::fmt::Debug + Encode + Clone;

#[cfg(test)]
mod test
{
    use super::*;
}
