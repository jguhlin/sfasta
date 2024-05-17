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
pub struct Sequence
{
    pub sequence: Option<Bytes>,
    pub scores: Option<Bytes>,
    pub header: Option<Bytes>,
    pub id: Option<Bytes>,
    /// Primarily used downstream, but when used for random access
    /// this is the offset from the start of the sequence
    pub offset: usize,
}

impl Sequence
{
    pub fn into_parts(
        self,
    ) -> (Option<Bytes>, Option<Bytes>, Option<Bytes>, Option<Bytes>)
    {
        {
            (self.id, self.header, self.sequence, self.scores)
        }
    }

    pub fn new(
        sequence: Option<Bytes>,
        id: Option<Bytes>,
        header: Option<Bytes>,
        scores: Option<Bytes>,
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

impl From<Vec<u8>> for Sequence
{
    fn from(seq: Vec<u8>) -> Sequence
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

impl From<Bytes> for Sequence
{
    fn from(seq: Bytes) -> Sequence
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
        let seq = Sequence::from(vec![b'A', b'C', b'G', b'T']);
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
        let mut seq = Sequence::from(vec![b'a', b'c', b'g', b't']);
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
        let seq = Sequence::from(vec![]);
        assert_eq!(seq.is_empty(), true);

        let seq = Sequence::from(vec![b'A']);
        assert_eq!(seq.is_empty(), false);

        // Test len
        let seq = Sequence::from(vec![b'A', b'C', b'G', b'T']);
        assert_eq!(seq.len(), 4);
    }
}
