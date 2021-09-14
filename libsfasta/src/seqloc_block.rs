use crate::structs::{default_compression_level, CompressionType};

use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use serde_bytes::ByteBuf;
use xz::read::{XzDecoder, XzEncoder};
