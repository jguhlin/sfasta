use crate::structs::{default_compression_level, CompressionType};

use std::io::{Read, Write};

use xz::read::{XzDecoder, XzEncoder};
