use super::Loc;
use flume::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};

pub mod bytes_block_store;
pub mod masking;
pub mod string_block_store;
pub mod threads;

pub use bytes_block_store::*;
pub use masking::*;
pub use string_block_store::*;
pub use threads::*;

pub(crate) type LocMutex = Arc<Mutex<(bool, Vec<Loc>)>>;
type Queue = (Sender<(LocMutex, Vec<u8>)>, Receiver<(LocMutex, Vec<u8>)>);

/// First is new data, second is shutdown
type Wakeup = Arc<(Mutex<(bool, bool)>, Condvar)>;
