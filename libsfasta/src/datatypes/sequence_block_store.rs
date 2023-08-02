//! A block store for storing sequences. This includes nucleotide, aminos, and scores.

use std::io::{Read, Seek, Write};

use simdutf8::basic::from_utf8;

use crate::compression::CompressionConfig;
use crate::datatypes::{BytesBlockStore, Loc};
use crate::CompressionType;

pub struct SequenceBlockStore {
    inner: BytesBlockStore,
}

impl Default for SequenceBlockStore {
    fn default() -> Self {
        let compression_config = CompressionConfig::new()
            .with_compression_type(CompressionType::ZSTD)
            .with_compression_level(3);

        SequenceBlockStore {
            inner: BytesBlockStore::default()
                .with_block_size(512 * 1024)
                .with_compression(compression_config),
        }
    }
}

impl SequenceBlockStore {
    pub fn with_block_size(mut self, block_size: usize) -> Self {
        self.inner = self.inner.with_block_size(block_size);
        self
    }

    pub fn add(&mut self, input: &str) -> Vec<Loc> {
        self.inner
            .add(input.as_bytes())
            .expect("Failed to add string to block store")
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> Option<u64>
    where
        W: Write + Seek,
    {
        self.inner.write_to_buffer(&mut out_buf)
    }

    pub fn from_buffer<R>(mut in_buf: &mut R, starting_pos: u64) -> Result<Self, String>
    where
        R: Read + Seek,
    {
        let inner = match BytesBlockStore::from_buffer(&mut in_buf, starting_pos) {
            Ok(inner) => inner,
            Err(e) => return Err(e),
        };

        let store = SequenceBlockStore { inner };
        Ok(store)
    }

    pub fn prefetch<R>(&mut self, in_buf: &mut R)
    where
        R: Read + Seek,
    {
        log::info!("Prefetching string block store");
        self.inner.prefetch(in_buf)
    }

    // TODO: Needed?
    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> Vec<u8>
    where
        R: Read + Seek,
    {
        self.inner.get_block(in_buf, block)
    }

    // TODO: Needed?
    pub fn get_block_uncached<R>(&mut self, mut in_buf: &mut R, block: u32) -> Vec<u8>
    where
        R: Read + Seek,
    {
        self.inner.get_block_uncached(&mut in_buf, block)
    }

    // TODO: Should be fallible...
    pub fn get<R>(&mut self, in_buf: &mut R, loc: &[Loc]) -> String
    where
        R: Read + Seek,
    {
        let string_as_bytes = self.inner.get(in_buf, loc);

        from_utf8(&string_as_bytes).unwrap().to_string()
    }

    pub fn get_loaded(&self, loc: &[Loc]) -> String {
        let string_as_bytes = self.inner.get_loaded(loc);
        from_utf8(&string_as_bytes).unwrap().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_add_id() {
        let mut store = SequenceBlockStore {
            ..Default::default()
        };

        let test_ids = vec![
            "Medtr5g026775.t1",
            "ARABIDOPSIS_SUPER_COOL_GENE",
            "ID WITH A SPACE EVEN THOUGH ITS INVALID",
            "same, but lowercase....",
        ];

        let mut locs = Vec::new();

        for id in test_ids.iter() {
            locs.push(store.add(id));
        }

        let mut buffer = Cursor::new(Vec::new());
        store.write_to_buffer(&mut buffer);
        let mut store = SequenceBlockStore::from_buffer(&mut buffer, 0).unwrap();

        for i in 0..test_ids.len() {
            let id = store.get(&mut buffer, &locs[i]);
            assert_eq!(id, test_ids[i]);
        }
    }
}
