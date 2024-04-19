/// no, delete
use super::*;

use flume::*;
use strum_macros::{EnumCount, EnumIter, FromRepr};

use std::sync::{atomic::AtomicBool, Arc, Condvar, Mutex};

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
pub(crate) type LocMutex = (Arc<bool>, Arc<Mutex<Vec<Loc>>>);

/// This is the placeholder type sent back to the submitted
///
/// AtomicBool marks completion status
/// Mutex<Vec<u64>> stores the entry in the SeqLocStore
/// (This is typically used for the ID index)
pub(crate) type SeqLocEntryMutex = (Arc<AtomicBool>, Arc<Mutex<Vec<u64>>>);

type Queue = (Sender<(LocMutex, Vec<u8>)>, Receiver<(LocMutex, Vec<u8>)>);
type SeqlocEntry = (
    Sender<(Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>)>,
    Receiver<(Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>, Vec<Loc>)>,
);

pub struct Coordinator
{
    // Coordinator receives Id, Header, Sequence, Scores, (& todo Signal, Modifications)
    // Masking is generated from Sequence
    receiver: (
        Sender<(LocMutex, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)>,
        Receiver<(LocMutex, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>)>,
    ),

    // Queues to the other worker threads
    ids_queue: Queue,
    headers_queue: Queue,
    sequence_queue: Queue,
    scores_queue: Queue,
    masking_queue: Queue,

    // Final queue
    seqlocs_queue: SeqlocEntry,

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

    // Condvar to signal threads to wake back up
    data_condvar: Arc<(Mutex<bool>, Condvar)>,

    // Queue size
    queue_size: usize,
}

impl Coordinator
{
    pub fn new(queue_size: usize) -> Self
    {
        let receiver = flume::bounded(queue_size);
        let ids_queue = flume::bounded(queue_size);
        let headers_queue = flume::bounded(queue_size);
        let sequence_queue = flume::bounded(queue_size);
        let scores_queue = flume::bounded(queue_size);
        let masking_queue = flume::bounded(queue_size);
        let seqlocs_queue = flume::bounded(queue_size);

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
            data_condvar: Arc::new((Mutex::new(false), Condvar::new())),
            queue_size,
        }
    }

    pub fn with_id_store(mut self, ids: StringBlockStore) -> Self
    {
        self.ids = Some(ids);
        self
    }

    pub fn with_header_store(mut self, headers: StringBlockStore) -> Self
    {
        self.headers = Some(headers);
        self
    }

    pub fn with_sequence_store(mut self, sequence: SequenceBlockStore) -> Self
    {
        self.sequence = Some(sequence);
        self
    }

    pub fn with_scores_store(mut self, scores: StringBlockStore) -> Self
    {
        self.scores = Some(scores);
        self
    }

    pub fn with_masking_store(mut self, masking: MaskingStoreBuilder) -> Self
    {
        self.masking = Some(masking);
        self
    }

    pub fn start(&self) {}
}
