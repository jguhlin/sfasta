use std::io::{Read, Seek, Write};

use simdutf8::basic::from_utf8;

use crate::datatypes::{BytesBlockStore, Loc};

#[derive(Clone)]
pub struct StringBlockStore {
    inner: BytesBlockStore,
}

impl Default for StringBlockStore {
    fn default() -> Self {
        StringBlockStore {
            inner: BytesBlockStore::default().with_block_size(2 * 1024 * 1024),
        }
    }
}

impl StringBlockStore {
    pub fn with_block_size(mut self, block_size: usize) -> Self {
        self.inner = self.inner.with_block_size(block_size);
        self
    }

    pub fn add<S: AsRef<str>>(&mut self, input: S) -> Vec<Loc> {
        self.inner.add(input.as_ref().as_bytes())
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

        let store = StringBlockStore { inner };
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
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_add_id() {
        let mut store = StringBlockStore {
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
        let mut store = StringBlockStore::from_buffer(&mut buffer, 0).unwrap();

        for i in 0..test_ids.len() {
            let id = store.get(&mut buffer, &locs[i]);
            assert_eq!(id, test_ids[i]);
        }
    }
}
