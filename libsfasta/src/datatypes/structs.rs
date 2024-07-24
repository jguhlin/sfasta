use std::{any::Any, borrow::BorrowMut, io::prelude::*};

use libcompression::*;

use bytes::Bytes;

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
pub struct SequenceMetadata
{
    pub id: Option<String>,
    pub length: u64,
    pub masking: bool,
    pub scores: bool,
    pub header: bool,
}


#[derive(PartialEq, Eq, Clone, Debug, Default)]
pub struct Sequence<T> {
    pub sequence: Option<T>,
    pub scores: Option<T>,
    pub header: Option<T>,
    pub id: Option<T>,
    pub offset: usize,
}

// Implementations for specific types if needed
impl Sequence<Bytes> {
    pub fn len(&self) -> usize {
        self.sequence.as_ref().unwrap().len()
    }

    pub fn make_uppercase(&mut self)
    {
        // Convert to Vec<u8>
        let mut seq: Vec<u8> = self.sequence.take().unwrap().into();
        seq.make_ascii_uppercase();
        self.sequence = Some(Bytes::from(seq));
    }

    pub fn make_lowercase(&mut self)
    {
        // Convert to Vec<u8>
        let mut seq: Vec<u8> = self.sequence.take().unwrap().into();
        seq.make_ascii_lowercase();
        self.sequence = Some(Bytes::from(seq));
    }

    pub fn is_empty(&self) -> bool
    {
        self.sequence.as_ref().unwrap().is_empty()
    }

}

impl Sequence<Vec<u8>> {
    pub fn len(&self) -> usize {
        self.sequence.as_ref().unwrap().len()
    }

    pub fn make_uppercase(&mut self)
    {
        // Convert to Vec<u8>
        let mut seq: Vec<u8> = self.sequence.take().unwrap().into();
        seq.make_ascii_uppercase();
        self.sequence = Some(seq);
    }

    pub fn make_lowercase(&mut self)
    {
        // Convert to Vec<u8>
        let mut seq: Vec<u8> = self.sequence.take().unwrap().into();
        seq.make_ascii_lowercase();
        self.sequence = Some(seq);
    }

    pub fn is_empty(&self) -> bool
    {
        self.sequence.as_ref().unwrap().is_empty()
    }
}

impl<T> Sequence<T>
{
    pub fn into_parts(
        self,
    ) -> (Option<T>, Option<T>, Option<T>, Option<T>)
    {
        {
            (self.id, self.header, self.sequence, self.scores)
        }
    }

    pub fn new(
        sequence: Option<T>,
        id: Option<T>,
        header: Option<T>,
        scores: Option<T>,
    ) -> Sequence<T>
    {
        Sequence {
            sequence,
            header,
            id,
            scores,
            offset: 0,
        }
    }

    


}

// Prefer conversion to Bytes
impl From<Vec<u8>> for Sequence<Bytes>
{
    fn from(seq: Vec<u8>) -> Sequence<Bytes>
    {
        Sequence {
            sequence: Some(Bytes::from(seq)),
            header: None,
            id: None,
            scores: None,
            offset: 0,
        }
    }
}

impl From<Bytes> for Sequence<Bytes>
{
    fn from(seq: Bytes) -> Sequence<Bytes>
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

impl From<Vec<u8>> for Sequence<Vec<u8>>
{
    fn from(seq: Vec<u8>) -> Sequence<Vec<u8>>
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

#[cfg(test)]
mod tests
{
    use super::*;

    #[test]
    fn test_sequence()
    {
        let seq: Sequence<Bytes> = Sequence::from(vec![b'A', b'C', b'G', b'T']);
        assert_eq!(
            seq.sequence.as_ref().unwrap(),
            &vec![b'A', b'C', b'G', b'T']
        );
        assert_eq!(seq.len(), 4);
        assert_eq!(seq.is_empty(), false);

        // Test into_parts
        let (id, header, sequence, scores) = seq.into_parts();
        assert_eq!(id, None);
        assert_eq!(header, None);
        assert_eq!(sequence, Some(Bytes::from(vec![b'A', b'C', b'G', b'T'])));
        assert_eq!(scores, None);

        // Fuller test
        let seq = Sequence::new(
            Some(Bytes::from(vec![b'A', b'C', b'G', b'T'])),
            Some(Bytes::from(vec![b'1', b'2', b'3'])),
            Some(Bytes::from(vec![b'4', b'5', b'6'])),
            Some(Bytes::from(vec![b'7', b'8', b'9'])),
        );

        assert_eq!(
            seq.sequence.as_ref().unwrap(),
            &vec![b'A', b'C', b'G', b'T']
        );
        assert_eq!(seq.id.as_ref().unwrap(), &vec![b'1', b'2', b'3']);
        assert_eq!(seq.header.as_ref().unwrap(), &vec![b'4', b'5', b'6']);
        assert_eq!(seq.scores.as_ref().unwrap(), &vec![b'7', b'8', b'9']);

        // Test into_parts
        let (id, header, sequence, scores) = seq.into_parts();
        assert_eq!(id, Some(Bytes::from(vec![b'1', b'2', b'3'])));
        assert_eq!(header, Some(Bytes::from(vec![b'4', b'5', b'6'])));
        assert_eq!(sequence, Some(Bytes::from(vec![b'A', b'C', b'G', b'T'])));
        assert_eq!(scores, Some(Bytes::from(vec![b'7', b'8', b'9'])));

        // Test make_uppercase and make_lowercase
        let mut seq: Sequence<Bytes> = Sequence::from(vec![b'a', b'c', b'g', b't']);
        seq.make_uppercase();
        assert_eq!(
            seq.sequence.as_ref().unwrap(),
            &vec![b'A', b'C', b'G', b'T']
        );
        seq.make_lowercase();
        assert_eq!(
            seq.sequence.as_ref().unwrap(),
            &vec![b'a', b'c', b'g', b't']
        );

        // Test is_empty
        let seq: Sequence<Bytes> = Sequence::from(vec![]);
        assert_eq!(seq.is_empty(), true);

        let seq: Sequence<Bytes> = Sequence::from(vec![b'A']);
        assert_eq!(seq.is_empty(), false);

        // Test len
        let seq: Sequence<Bytes> = Sequence::from(vec![b'A', b'C', b'G', b'T']);
        assert_eq!(seq.len(), 4);
    }
}
