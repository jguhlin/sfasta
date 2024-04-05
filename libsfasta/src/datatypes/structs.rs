use std::{any::Any, io::prelude::*};

use libcompression::*;

// SuperTrait -- needed for pyO3
pub trait ReadAndSeek: Read + Seek + BufRead {}
impl<T: Read + Seek + BufRead> ReadAndSeek for T {}

pub trait ReadAndSeekAndSend: Read + Seek {}
impl<T: Read + Seek> ReadAndSeekAndSend for T {}

pub trait WriteAndSeek: Write + Seek {}
impl<T: Write + Seek + Any> WriteAndSeek for T {}

pub trait T: Any {}
impl T for dyn WriteAndSeek {}

#[derive(PartialEq, Eq)]
pub enum SeqMode
{
    Linear,
    Random,
}

#[derive(PartialEq, Eq, Debug, Clone)]
pub struct Header
{
    pub id: Option<String>,
    pub comment: Option<String>,
    pub citation: Option<String>,
    pub compression_type: CompressionType,
}

#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Sequence
{
    pub sequence: Option<Vec<u8>>,
    pub scores: Option<Vec<u8>>,
    pub header: Option<String>,
    pub id: Option<String>,
    /// Primarily used downstream, but when used for random access this is the offset from the start of the sequence
    pub offset: usize,
}

impl Sequence
{
    pub fn into_parts(self) -> (Option<String>, Option<String>, Option<Vec<u8>>, Option<Vec<u8>>)
    {
        {
            (self.id, self.header, self.sequence, self.scores)
        }
    }

    pub fn new(
        sequence: Option<Vec<u8>>,
        id: Option<String>,
        header: Option<String>,
        scores: Option<Vec<u8>>,
    ) -> Sequence
    {
        Sequence {
            sequence,
            header,
            id,
            scores,
            offset: 0,
        }
    }

    pub fn len(&self) -> usize
    {
        self.sequence.as_ref().unwrap().len()
    }

    pub fn make_uppercase(&mut self)
    {
        self.sequence.as_mut().unwrap().make_ascii_uppercase();
    }

    pub fn make_lowercase(&mut self)
    {
        self.sequence.as_mut().unwrap().make_ascii_lowercase();
    }

    pub fn is_empty(&self) -> bool
    {
        self.sequence.as_ref().unwrap().is_empty()
    }
}

impl From<Vec<u8>> for Sequence
{
    fn from(seq: Vec<u8>) -> Sequence
    {
        Sequence {
            sequence: Some(seq),
            header: None,
            id: None,
            scores: None,
            offset: 0,
        }
    }
}
