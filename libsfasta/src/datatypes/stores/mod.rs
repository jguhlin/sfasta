use super::Loc;
use flume::{Receiver, Sender};
use std::sync::{atomic::AtomicBool, Arc, Mutex};

pub mod bytes_block_store;
pub mod masking;
pub mod string_block_store;
pub mod threads;

pub use bytes_block_store::*;
pub use masking::*;
pub use string_block_store::*;
pub use threads::*;

pub(crate) type LocMutex = Arc<(AtomicBool, Mutex<Vec<Loc>>)>;
