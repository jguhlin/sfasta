// TODO: Sequences should be like this
// TODO: FASTQ scores should be like this (well, more like Sequences, since too big to fit in memory)

use std::io::{Read, Seek, SeekFrom, Write};

use crate::data_types::{zstd_encoder, CompressionType, Loc};

pub struct Headers {
    location: u64,
    block_index_pos: u64,
    block_locations: Option<Vec<u64>>,
    block_size: usize,
    pub data: Option<Vec<u8>>, // Only used for writing...
    pub compression_type: CompressionType,
    cache: Option<(u32, Vec<u8>)>,
}

impl Default for Headers {
    fn default() -> Self {
        Headers {
            location: 0,
            block_index_pos: 0,
            block_locations: None,
            block_size: 2 * 1024 * 1024,
            data: None,
            compression_type: CompressionType::ZSTD,
            cache: None,
        }
    }
}

impl Headers {
    pub fn add_header(&mut self, header: String) -> Vec<Loc> {
        if self.data.is_none() {
            self.data = Some(Vec::with_capacity(self.block_size));
        }

        let data = self.data.as_mut().unwrap();

        let mut start = data.len();
        data.extend(header.as_bytes());
        let end = data.len() - 1;
        let starting_block = start / self.block_size;
        let ending_block = end / self.block_size;

        let mut locs = Vec::new();

        let len = header.len() as u32;

        for block in starting_block..=ending_block {
            let block_start = start % self.block_size;
            let block_end = if block == ending_block {
                end % self.block_size
            } else {
                self.block_size - 1
            };
            start = block_end + 1;
            locs.push(Loc::new(block as u32, block_start as u32, block_end as u32));
        }

        locs
    }

    fn emit_blocks(&mut self) -> Vec<Vec<u8>> {
        let data = self.data.take().unwrap();
        let mut blocks = Vec::new();
        let len = data.len();

        for i in (0..len).step_by(self.block_size) {
            let end = std::cmp::min(i + self.block_size, data.len());
            blocks.push(data[i..end].to_vec());
        }

        blocks
    }

    pub fn write_to_buffer<W>(&mut self, mut out_buf: &mut W) -> Option<u64>
    where
        W: Write + Seek,
    {
        if self.data.is_none() {
            return None;
        }

        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut block_locations_pos: u64 = 0;

        let starting_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
        // TODO: This is a lie, only zstd is supported as of right now...
        bincode::encode_into_std_write(&self.compression_type, &mut out_buf, bincode_config)
            .unwrap();
        bincode::encode_into_std_write(&block_locations_pos, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.block_size, &mut out_buf, bincode_config).unwrap();

        let blocks = self.emit_blocks();

        let mut block_locations = Vec::new();

        let mut compressor = zstd_encoder(3);

        for block in blocks {
            let block_start = out_buf.seek(SeekFrom::Current(0)).unwrap();
            let compressed_block = compressor.compress(&block).unwrap();
            bincode::encode_into_std_write(&compressed_block, &mut out_buf, bincode_config)
                .unwrap();
            block_locations.push(block_start);
        }

        block_locations_pos = out_buf.seek(SeekFrom::Current(0)).unwrap();
        bincode::encode_into_std_write(&block_locations, &mut out_buf, bincode_config).unwrap();
        self.block_locations = Some(block_locations); // Probably not needed...

        let end = out_buf.seek(SeekFrom::Current(0)).unwrap();
        out_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        bincode::encode_into_std_write(&self.compression_type, &mut out_buf, bincode_config)
            .unwrap();
        bincode::encode_into_std_write(&block_locations_pos, &mut out_buf, bincode_config).unwrap();
        bincode::encode_into_std_write(&self.block_size, &mut out_buf, bincode_config).unwrap();

        // Back to the end so we don't interfere with anything...
        out_buf.seek(SeekFrom::Start(end)).unwrap();

        Some(starting_pos)
    }

    pub fn from_buffer<R>(mut in_buf: &mut R, starting_pos: u64) -> Self
    where
        R: Read + Seek,
    {
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();

        let mut headers = Headers::default();

        in_buf.seek(SeekFrom::Start(starting_pos)).unwrap();
        headers.compression_type =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        headers.block_index_pos =
            bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        headers.block_size = bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
        headers.location = starting_pos;

        in_buf
            .seek(SeekFrom::Start(headers.block_index_pos))
            .unwrap();
        headers.block_locations =
            Some(bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap());

        headers
    }

    pub fn get_header<R>(&mut self, mut in_buf: &mut R, loc: &[Loc]) -> String
    where
        R: Read + Seek,
    {
        let mut header = String::with_capacity(1024);
        let bincode_config = bincode::config::standard().with_fixed_int_encoding();
        let block_locations = self.block_locations.as_ref().unwrap();

        let mut decompressor = zstd::bulk::Decompressor::new().unwrap();
        decompressor.include_magicbytes(false).unwrap();

        if self.cache.is_some() {
            let mut cache = self.cache.as_mut().unwrap();
            for i in loc {
                if i.block == cache.0 {
                    let start = i.start as usize;
                    let end = i.end as usize;
                    header.push_str(std::str::from_utf8(&cache.1[start..=end]).unwrap());
                } else {
                    let block_location = block_locations[i.block as usize];
                    in_buf.seek(SeekFrom::Start(block_location)).unwrap();
                    let compressed_block: Vec<u8> =
                        bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
                    let decompressed_block = decompressor
                        .decompress(&compressed_block, self.block_size)
                        .unwrap();
                    let start = i.start as usize;
                    let end = i.end as usize;
                    header.push_str(std::str::from_utf8(&decompressed_block[start..=end]).unwrap());
                    *cache = (i.block, decompressed_block);
                }
            }
        } else {
            // TODO: Repetitive code...
            for i in loc {
                let block_location = block_locations[i.block as usize];
                in_buf.seek(SeekFrom::Start(block_location)).unwrap();
                let compressed_block: Vec<u8> =
                    bincode::decode_from_std_read(&mut in_buf, bincode_config).unwrap();
                let decompressed_block = decompressor
                    .decompress(&compressed_block, self.block_size)
                    .unwrap();
                let start = i.start as usize;
                let end = i.end as usize;
                header.push_str(std::str::from_utf8(&decompressed_block[start..=end]).unwrap());
                self.cache = Some((i.block, decompressed_block));
            }
        }

        for i in loc {}

        header
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_add_header() {
        let mut headers = Headers {
            block_size: 10,
            ..Default::default()
        };

        let test_headers =
            vec!["This is my first header",
        "second header",
        "THIRD HEADER WOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOOW",
        "fourth header"];

        let mut locs = Vec::new();

        for header in test_headers.iter() {
            locs.push(headers.add_header(header.to_string()));
        }

        let mut buffer = Cursor::new(Vec::new());
        headers.write_to_buffer(&mut buffer);
        let mut headers = Headers::from_buffer(&mut buffer, 0);

        for i in 0..test_headers.len() {
            let header = headers.get_header(&mut buffer, &locs[i]);
            assert_eq!(header, test_headers[i]);
        }
    }
}
