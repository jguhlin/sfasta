use super::*;

use strum_macros::{EnumCount, EnumIter, FromRepr};
use flume::prelude::*;

use std::sync::{Arc, AtomicBool, Mutex};

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

/// This is the placeholder type sent out to the worker threads
/// AtomicBool to mark completion status
/// Mutex<Vec<Loc>> to store the data
/// 
/// When all are complete, the data is sent to the final queue (SeqLocsStore)
pub(crate) type LocMutex = (Arc<AtomicBool>, Arc<Mutex<Vec<Loc>>>);

/// This is the placeholder type sent back to the submitted
/// 
/// AtomicBool marks completion status
/// Mutex<Vec<u64>> stores the entry in the SeqLocStore
/// (This is typically used for the ID index)
pub(crate) type SeqLocEntryMutex = (Arc<AtomicBool>, Arc<Mutex<Vec<u64>>>); 

pub struct Coordinator {
    // Coordinator receives Id, Header, Sequence, Scores, (& todo Signal, Modifications)
    // Masking is generated from Sequence
    receiver: Reader<(LocMutex, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)>,

    // Queues to the other worker threads
    ids_queue: Sender<(LocMutex, Vec<u8>)>,
    headers_queue: Sender<(LocMutex, Vec<u8>)>,
    sequence_queue: Sender<(LocMutex, Vec<u8>)>,
    scores_queue: Sender<(LocMutex, Vec<u8>)>,
    masking_queue: Sender<(LocMutex, Vec<u8>)>,

    // Final queue
    seqlocs_queue: Sender<(Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>)>,

    // Place to hold while we wait for all LocMutex to complete
    // These are in order (Ids, Headers, Sequence, Scores, Masking)
    queue: Vec<(LocMutex, LocMutex, LocMutex, LocMutex, LocMutex)>, // todo other fields

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