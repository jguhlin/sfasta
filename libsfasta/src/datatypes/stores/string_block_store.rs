// todo don't know if I can justify this anymore, maybe just use
// BytesBlockStore direct now?

use std::{
    io::{BufRead, Read, Seek, Write},
    sync::Arc,
};

use simdutf8::basic::from_utf8;

use super::Builder;
use crate::datatypes::{
    BlockStoreError, BytesBlockStore, BytesBlockStoreBuilder, Loc,
};
use libcompression::*;

#[cfg(feature = "async")]
use libfilehandlemanager::AsyncFileHandleManager;

#[cfg(feature = "async")]
use crate::datatypes::DataOrLater;

#[cfg(feature = "async")]
use crate::parser::async_parser::{
    bincode_decode_from_buffer_async,
    bincode_decode_from_buffer_async_with_size_hint,
};

#[cfg(feature = "async")]
use bytes::{BufMut, Bytes, BytesMut};

#[cfg(feature = "async")]
use tokio::{
    fs::File,
    io::{AsyncSeekExt, BufReader},
    sync::{OwnedRwLockWriteGuard, RwLock},
};

#[cfg(feature = "async")]
use tokio_stream::StreamExt;

#[cfg(feature = "async")]
use async_stream::stream;

#[cfg(feature = "async")]
use tokio_stream::Stream;

pub struct StringBlockStoreBuilder
{
    inner: BytesBlockStoreBuilder,
}

impl Default for StringBlockStoreBuilder
{
    fn default() -> Self
    {
        StringBlockStoreBuilder {
            inner: BytesBlockStoreBuilder::default()
                .with_block_size(512 * 1024)
                .with_compression(CompressionConfig {
                    compression_type: CompressionType::ZSTD,
                    compression_level: 6,
                    compression_dict: None,
                }),
        }
    }
}

impl Builder<Vec<u8>> for StringBlockStoreBuilder
{
    fn add(&mut self, input: Vec<u8>) -> Result<Vec<Loc>, &str>
    {
        Ok(self
            .inner
            .add(input)
            .expect("Failed to add string to block store"))
    }

    fn finalize(&mut self) -> Result<(), &str>
    {
        match self.inner.finalize() {
            Ok(_) => Ok(()),
            Err(e) => Err("Unable to finalize string block store"),
        }
    }
}

impl StringBlockStoreBuilder
{
    pub fn with_dict(mut self) -> Self
    {
        self.inner = self.inner.with_dict();
        self
    }

    pub fn with_dict_size(mut self, dict_size: u64) -> Self
    {
        self.inner = self.inner.with_dict_size(dict_size);
        self
    }

    pub fn with_dict_samples(mut self, dict_samples: u64) -> Self
    {
        self.inner = self.inner.with_dict_samples(dict_samples);
        self
    }

    pub fn with_compression(mut self, config: CompressionConfig) -> Self
    {
        self.inner = self.inner.with_compression(config);
        self
    }

    pub fn with_tree_compression(
        mut self,
        tree_compression: CompressionConfig,
    ) -> Self
    {
        self.inner = self.inner.with_tree_compression(tree_compression);
        self
    }

    pub fn with_compression_worker(
        mut self,
        compression_worker: Arc<CompressionWorker>,
    ) -> Self
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

    pub fn write_block_locations<W>(
        &mut self,
        mut out_buf: W,
    ) -> Result<(), BlockStoreError>
    where
        W: Write + Seek,
    {
        self.inner.write_block_locations(&mut out_buf)
    }

    pub fn block_len(&self) -> usize
    {
        self.inner.block_len()
    }

    pub fn finalize(&mut self)
    {
        self.inner.finalize();
    }

    pub fn with_block_size(mut self, block_size: usize) -> Self
    {
        self.inner = self.inner.with_block_size(block_size);
        self
    }

    pub fn add(&mut self, input: Vec<u8>) -> Vec<Loc>
    {
        self.inner
            .add(input)
            .expect("Failed to add string to block store")
    }
}

pub struct StringBlockStore
{
    #[cfg(feature = "async")]
    inner: Arc<BytesBlockStore>,

    #[cfg(not(feature = "async"))]
    inner: BytesBlockStore,
}

impl StringBlockStore
{
    #[cfg(not(feature = "async"))]
    pub fn from_buffer<R>(
        mut in_buf: &mut R,
        starting_pos: u64,
    ) -> Result<Self, String>
    where
        R: Read + Seek + Send + Sync + BufRead,
    {
        let inner =
            match BytesBlockStore::from_buffer(&mut in_buf, starting_pos) {
                Ok(inner) => inner,
                Err(e) => return Err(e),
            };

        let store = StringBlockStore { inner };
        Ok(store)
    }

    #[cfg(feature = "async")]
    pub async fn stream(
        self: Arc<Self>,
        fhm: Arc<AsyncFileHandleManager>,
    ) -> impl Stream<Item = (u32, Bytes)>
    {
        Arc::clone(&self.inner).stream(fhm).await
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(in_buf))]
    pub async fn from_buffer(
        mut in_buf: &mut tokio::io::BufReader<tokio::fs::File>,
        filename: String,
        starting_pos: u64,
    ) -> Result<Self, String>
    {
        let inner = match BytesBlockStore::from_buffer(
            &mut in_buf,
            filename,
            starting_pos,
        )
        .await
        {
            Ok(inner) => inner,
            Err(e) => return Err(e),
        };

        let store = StringBlockStore {
            inner: Arc::new(inner),
        };
        Ok(store)
    }

    #[cfg(not(feature = "async"))]
    // TODO: Needed?
    pub fn get_block<R>(&mut self, in_buf: &mut R, block: u32) -> Vec<u8>
    where
        R: Read + Seek + Send + Sync,
    {
        self.inner.get_block(in_buf, block)
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self, in_buf))]
    pub async fn get_block(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<BufReader<File>>,
        block: u32,
    ) -> Bytes
    {
        match self.inner.get_block(in_buf, block).await {
            DataOrLater::Data(data) => data,
            DataOrLater::Later(data) => data.await.unwrap(),
        }
    }

    #[cfg(not(feature = "async"))]
    pub fn get_block_uncached<R>(
        &mut self,
        mut in_buf: &mut R,
        block: u32,
        buffer: &mut [u8],
    ) where
        R: Read + Seek + Send + Sync,
    {
        self.inner.get_block_uncached(&mut in_buf, block, buffer)
    }

    #[cfg(not(feature = "async"))]
    // todo should be falliable
    pub fn get<R>(&mut self, in_buf: &mut R, loc: &[Loc]) -> bytes::Bytes
    where
        R: Read + Seek + Send + Sync,
    {
        self.inner.get(in_buf, loc)
        // let string_as_bytes = self.inner.get(in_buf, loc);
        // from_utf8(&string_as_bytes).unwrap().to_string()
    }

    #[cfg(feature = "async")]
    #[tracing::instrument(skip(self, in_buf))]
    pub async fn get(
        &self,
        in_buf: &mut tokio::sync::OwnedMutexGuard<BufReader<File>>,
        loc: &[Loc],
    ) -> Bytes
    {
        self.inner.get(in_buf, loc).await
        // string_as_bytes = self.inner.get(in_buf, loc).await
        // from_utf8(&string_as_bytes).unwrap().to_string()
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
        let mut store = StringBlockStoreBuilder {
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
            locs.push(store.add(id.as_bytes().to_vec()));
        }
    }
}
