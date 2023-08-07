use criterion::{black_box, criterion_group, criterion_main, Criterion};

use libsfasta::_compression_stream_buffer::CompressionStreamBuffer;

use rand::Rng;

use std::io::Cursor;
use std::sync::Arc;
