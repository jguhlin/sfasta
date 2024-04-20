use super::Loc;
use flume::{Receiver, Sender};
use std::sync::{Arc, Mutex};

pub mod bytes_block_store;
pub mod masking;
pub mod string_block_store;
pub mod threads;

pub use bytes_block_store::*;
pub use masking::*;
pub use string_block_store::*;
pub use threads::*;

type Queue = (Sender<(LocMutex, Vec<u8>)>, Receiver<(LocMutex, Vec<u8>)>);

pub(crate) type LocMutex = Arc<Mutex<(bool, Vec<Loc>)>>;
