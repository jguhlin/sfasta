//! A block store for storing sequences. This includes nucleotide, aminos, and scores.

// TODO: Since this doesn't add any special functionality, should this just be replaced with BytesBlockStore? Maybe an
// alias?

use std::{
    io::{Read, Seek, Write},
    sync::Arc,
};

use simdutf8::basic::from_utf8;

use crate::datatypes::{BytesBlockStore, BytesBlockStoreBuilder, Loc};
use libcompression::*;

pub struct SequenceBlockStoreBuilder
{
    inner: BytesBlockStoreBuilder,
}

impl Default for SequenceBlockStoreBuilder
{
    fn default() -> Self
    {
        let compression_config = CompressionConfig::new()
            .with_compression_type(CompressionType::ZSTD)
            .with_compression_level(3);

        SequenceBlockStoreBuilder {
            inner: BytesBlockStoreBuilder::default()
                .with_block_size(512 * 1024)
                .with_compression(compression_config),
        }
    }
}

impl SequenceBlockStoreBuilder
{
    pub fn with_compression_worker(mut self, compression_worker: Arc<CompressionWorker>) -> Self
    {
        self.inner = self.inner.with_compression_worker(compression_worker);
        self
    }

    pub fn write_header<W>(&mut self, pos: u64, mut out_buf: &mut W)
    where
        W: Write + Seek,
    {
        self.inner.write_header(pos, &mut out_buf);
    }

    pub fn write_block_locations(&mut self)
    {
        self.inner.write_block_locations();
    }

    pub fn with_block_size(mut self, block_size: usize) -> Self
    {
        self.inner = self.inner.with_block_size(block_size);
        self
    }

    pub fn add(&mut self, input: &[u8]) -> Vec<Loc>
    {
        self.inner.add(input).expect("Failed to add string to block store")
    }

    pub fn finalize(&mut self)
    {
        self.inner.finalize();
    }
}

pub struct SequenceBlockStore
{
    inner: BytesBlockStore,
}

impl SequenceBlockStore
{
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

    pub fn get_loaded(&self, loc: &[Loc]) -> String
    {
        let string_as_bytes = self.inner.get_loaded(loc);
        from_utf8(&string_as_bytes).unwrap().to_string()
    }
}

#[cfg(test)]
mod tests
{
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_add_id()
    {
        let mut store = SequenceBlockStoreBuilder { ..Default::default() };

        let test_ids = vec![
            "Medtr5g026775.t1",
            "ARABIDOPSIS_SUPER_COOL_GENE",
            "ID WITH A SPACE EVEN THOUGH ITS INVALID",
            "same, but lowercase....",
        ];

        let mut locs = Vec::new();

        for id in test_ids.iter() {
            locs.push(store.add(id.as_bytes()));
        }
    }
}
