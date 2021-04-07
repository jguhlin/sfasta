use crate::sequence_block::*;

#[derive(Default)]
pub struct SequenceBuffer {
    block_size: u32,
    cur_block_id: u32,
    blocks: Vec<SequenceBlockCompressed>, // TODO: Replace with crossbeam queue or something...
    buffer: Vec<u8>,
}

impl Drop for SequenceBuffer {
    fn drop(&mut self) {
        assert!(self.buffer.len() == 0, "SequenceBuffer was not empty. Finalize the buffer to emit the final block.");
    }
}

impl SequenceBuffer {
    pub fn add_sequence(&mut self, x: &[u8]) -> Result<Vec<(u32, (usize, usize))>, &'static str> {
        assert!(self.block_size > 0);

        let mut locs = Vec::new();

        let block_size = self.block_size as usize;

        let mut seq = &x[..];

        while seq.len() > 0 {
            let len = self.len();

            let mut end = seq.len();

            // Sequence will run past the end of the block...
            if len + seq.len() >= block_size {
                end = block_size - len;
            }

            self.buffer.extend_from_slice(&seq[0..end]);

            println!("Adding loc... {:#?}", locs);

            locs.push((self.cur_block_id, (len, len + end)));

            if self.len() >= block_size {
                self.emit_block();
            }

            seq = &seq[end..];
        }

        Ok(locs)
    }

    pub fn emit_block(&mut self) {
        let newbuf = Vec::with_capacity(self.block_size as usize);

        let seq = std::mem::replace(&mut self.buffer, newbuf);

        let x = SequenceBlock {
            seq,
            ..Default::default()
        };

        let x = x.compress();
        self.blocks.push(x);
        self.cur_block_id += 1;
    }

    // Convenience functions

    fn len(&mut self) -> usize {
        self.buffer.len()
    }

    pub fn with_blocksize(mut self, block_size: u32) -> Self {
        self.block_size = block_size;
        self.buffer = Vec::with_capacity(block_size as usize);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    pub fn test_add_sequence() {
        let myseq = b"ACTGGGGGGGG".to_vec();

        let test_block_size = 512 * 1024;
        
        let mut sb = SequenceBuffer::default().with_blocksize(test_block_size);
        let mut locs = Vec::new();
        
        let loc = sb.add_sequence(&myseq[..]).unwrap();
        locs.extend(loc);

        let mut largeseq = Vec::new();
        while largeseq.len() <= 4 * 1024 * 1024 {
            largeseq.extend_from_slice(&myseq[..]);
        }

        let loc = sb.add_sequence(&largeseq).unwrap();
        locs.extend(loc);

        // THIS tests all emitted blocks... it DOES NOT test the "final" block
        // TODO: Probably a bad test once this is working with crossbeam stuff..
        println!("Block ID: {:#?}", sb.cur_block_id);
        for i in &sb.blocks {
            let j = i.decompress();
            assert!(j.seq.len() == test_block_size as usize);
        }

        sb.emit_block();

        assert!(locs.len() == 10);
    }

    #[test]
    #[should_panic(expected = "SequenceBuffer was not empty. Finalize the buffer to emit the final block.")]
    pub fn test_lingering_sequence_in_seqbuf() {
        let myseq = b"ACTGGGGGGGG".to_vec();

        let test_block_size = 512 * 1024;
        
        let mut sb = SequenceBuffer::default().with_blocksize(test_block_size);
        
        sb.add_sequence(&myseq[..]).expect("Error adding sequence");
    }
}