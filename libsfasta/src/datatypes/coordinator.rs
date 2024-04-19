use super::*;

use flume::*;
use strum_macros::{EnumCount, EnumIter, FromRepr};

use std::sync::{atomic::AtomicBool, Arc, Mutex};

#[derive(EnumIter, FromRepr, Debug, PartialEq, Eq, PartialOrd, Ord, EnumCount)]
pub enum DataTypes
{
    Ids,
    Headers,
    Sequence,
    Scores, // todo
    Masking,
    SeqLocs,
    Signal,        // todo
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

pub struct Coordinator
{
    // Coordinator receives Id, Header, Sequence, Scores, (& todo Signal, Modifications)
    // Masking is generated from Sequence
    receiver: Receiver<(LocMutex, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)>,

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

    // Stores
    // Temporary ownership of the stores
    ids: Option<StringBlockStore>,
    headers: Option<StringBlockStore>,
    sequence: Option<SequenceBlockStore>,
    scores: Option<StringBlockStore>,
    masking: Option<MaskingStoreBuilder>,
}

impl Coordinator
{
    pub fn new() -> Self
    {
        // definitely need to use bounded queues
        let (sender, receiver) = flume::unbounded();
        let (ids_queue, ids_receiver) = flume::unbounded();
        let (headers_queue, headers_receiver) = flume::unbounded();
        let (sequence_queue, sequence_receiver) = flume::unbounded();
        let (scores_queue, scores_receiver) = flume::unbounded();
        let (masking_queue, masking_receiver) = flume::unbounded();
        let (seqlocs_queue, seqlocs_receiver) = flume::unbounded();

        Self {
            receiver,
            ids_queue,
            headers_queue,
            sequence_queue,
            scores_queue,
            masking_queue,
            seqlocs_queue,
            queue: Vec::new(),
            ids: None,
            headers: None,
            sequence: None,
            scores: None,
            masking: None,
        }
    }

    pub fn start(&self) {}
}
