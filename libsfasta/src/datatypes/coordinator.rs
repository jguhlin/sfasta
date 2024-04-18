use super::*;

use strum_macros::{EnumCount, EnumIter, FromRepr};
use flume::prelude::*;

#[derive(EnumIter, FromRepr, Debug, PartialEq, Eq, PartialOrd, Ord, EnumCount)]
pub enum DataTypes {
    Ids,
    Headers,
    Sequence,
    Scores, // todo
    Masking,
    SeqLocs,
    Signal, // todo
    Modifications, // todo
}

pub struct Coordinator {
    // Coordinator receives Id, Header, Sequence, Scores, (& todo Signal, Modifications)
    // Masking is generated from Sequence
    receiver: Reader<(Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)>,
    ids_queue: Sender<Vec<u8>>,
    headers_queue: Sender<Vec<u8>>,
    sequence_queue: Sender<Vec<u8>>,
    scores_queue: Sender<Vec<u8>>,
    masking_queue: Sender<Vec<u8>>,
    seqlocs_queue: Sender<(Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>)>,

    
}

impl Coordinator {
    pub fn new() -> Self {
        Coordinator {}
    }

    pub fn start(&self) {

        
    }
}

let mut headers = StringBlockStoreBuilder::default()
    .with_block_size(block_size as usize)
    .with_compression_worker(Arc::clone(&compression_workers_thread));

let mut seqlocs = SeqLocsStoreBuilder::default();

let mut ids = StringBlockStoreBuilder::default()
    .with_block_size(block_size as usize)
    .with_compression_worker(Arc::clone(&compression_workers_thread));

let mut masking = MaskingStoreBuilder::default()
    .with_compression_worker(Arc::clone(&compression_workers_thread))
    .with_block_size(block_size as usize);

let mut sequences = SequenceBlockStoreBuilder::default()
    .with_block_size(block_size as usize)
    .with_compression_worker(Arc::clone(&compression_workers_thread));