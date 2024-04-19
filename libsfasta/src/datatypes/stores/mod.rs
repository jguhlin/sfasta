use std::sync::{Arc, Mutex};
use flume::{Sender, Receiver};
use super::Loc;

pub mod string_block_store;
pub mod bytes_block_store;
pub mod masking;

pub use bytes_block_store::*;
pub use string_block_store::*;
pub use masking::*;

pub(crate) type LocMutex = (Arc<bool>, Arc<Mutex<Vec<Loc>>>);
type Queue = (Sender<(LocMutex, Vec<u8>)>, Receiver<(LocMutex, Vec<u8>)>);