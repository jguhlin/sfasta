use std::io::{BufReader, Read, Seek};

use libfractaltree::FractalTreeDisk;

use crate::{datatypes::*, formats::*};

// todo redundant as this is also in the async_parser.rs
const SFASTA_MARKER: &[u8; 6] = b"sfasta";
const FULL_HEADER_SIZE: usize =
    6 + std::mem::size_of::<u64>() + std::mem::size_of::<DirectoryOnDisk>();

/// Open a SFASTA file from a buffer.
/// This gives ownership of the buffer to the SFASTA struct.
pub fn open_with_buffer<'sfa, R>(mut in_buf: R) -> Result<Sfasta<'sfa>, String>
where
    R: 'sfa + Read + Seek + Send + Sync,
{
    let sfasta = match open_from_buffer(&mut in_buf) {
        Ok(x) => x,
        Err(x) => return Result::Err(x),
    };

    let buf_size = get_reasonable_buffer_size(
        sfasta.sequenceblocks.as_ref().unwrap().block_size(),
    );

    let in_buf = BufReader::with_capacity(buf_size, in_buf);

    let sfasta = sfasta.with_buffer(in_buf);

    Ok(sfasta)
}

fn get_reasonable_buffer_size(buf_size: usize) -> usize
{
    // Do some logic to get a reasonable (power of 2)
    // buffer size based on the block size
    let buf_size = buf_size as f64 * 0.75; // Assume some compression efficiency
    let buf_size = buf_size as usize;
    let buf_size = buf_size.next_power_of_two();

    // Clamp to min and max sizes
    let buf_size = std::cmp::max(64 * 1024, buf_size);
    let buf_size = std::cmp::min(8 * 1024 * 1024, buf_size);
    buf_size
}

/// Open a SFASTA file from a buffer.
///
/// This does not take ownership of the buffer.
/// Useful for cloning the sfasta struct and creating new buffers
/// manually.
pub fn open_from_buffer<'sfa, R>(in_buf: &mut R) -> Result<Sfasta<'sfa>, String>
where
    R: 'sfa + Read + Seek + Send + Sync,
{
    let bincode_config_fixed = crate::BINCODE_CONFIG
        .with_fixed_int_encoding()
        .with_limit::<{ 2 * 1024 * 1024 }>();

    let mut sfasta_header: [u8; FULL_HEADER_SIZE] = [0; FULL_HEADER_SIZE];
    match in_buf.read_exact(&mut sfasta_header) {
        Ok(_) => (),
        Err(x) => {
            return Result::Err(format!(
            "Invalid buffer. Buffer too short. SFASTA marker is missing. {x}"
        ))
        }
    };

    if sfasta_header[0..6] != *SFASTA_MARKER {
        return Result::Err(
            "Invalid buffer. SFASTA marker is missing.".to_string(),
        );
    }

    let header: (u64, DirectoryOnDisk) = match bincode::decode_from_slice(
        &sfasta_header[6..],
        bincode_config_fixed,
    ) {
        Ok(x) => x.0,
        Err(x) => {
            return Result::Err(format!(
                "Invalid buffer. Failed to decode main header. {x}"
            ))
        }
    };

    let (version, directory) = header;

    if version != 1 {
        return Result::Err(format!(
            "Invalid buffer. Unsupported version: {version}"
        ));
    }

    // Note: dropped the length check, it was mostly for fuzzing, but
    // should be fine without it if we make sure to check everything
    // as we go

    let directory: Directory = directory.into();

    // Preload the seqlocs and index

    let mut in_buf = BufReader::new(in_buf);

    let seqlocs: Option<SeqLocsStore> = match SeqLocsStore::from_existing(
        directory.seqlocs_loc.unwrap().get(),
        &mut in_buf,
    ) {
        Ok(x) => Some(x),
        Err(x) => {
            return Result::Err(format!(
                "Invalid buffer. Failed to read seqlocs. {x}"
            ))
        }
    };

    let index = if directory.index_loc.is_some() {
        match FractalTreeDisk::from_buffer(
            &mut in_buf,
            directory.index_loc.unwrap().get(),
        ) {
            Ok(x) => Some(x),
            Err(x) => {
                return Result::Err(format!(
                    "Invalid buffer. Failed to read index. {x}"
                ))
            }
        }
    } else {
        None
    };

    let sequenceblocks = if directory.sequences_loc.is_some() {
        match BytesBlockStore::from_buffer(
            &mut in_buf,
            directory.sequences_loc.unwrap().get(),
        ) {
            Ok(x) => Some(x),
            Err(x) => {
                return Result::Err(format!(
                    "Invalid buffer. Failed to read sequenceblocks. {x}"
                ))
            }
        }
    } else {
        None
    };

    let ids = if directory.ids_loc.is_some() {
        match StringBlockStore::from_buffer(
            &mut in_buf,
            directory.ids_loc.unwrap().get(),
        ) {
            Ok(x) => Some(x),
            Err(x) => {
                return Result::Err(format!(
                    "Invalid buffer. Failed to read ids. {x}"
                ))
            }
        }
    } else {
        None
    };

    let headers = if directory.headers_loc.is_some() {
        match StringBlockStore::from_buffer(
            &mut in_buf,
            directory.headers_loc.unwrap().get(),
        ) {
            Ok(x) => Some(x),
            Err(x) => {
                return Result::Err(format!(
                    "Invalid buffer. Failed to read headers. {x}"
                ))
            }
        }
    } else {
        None
    };

    let masking = if directory.masking_loc.is_some() {
        match Masking::from_buffer(
            &mut in_buf,
            directory.masking_loc.unwrap().get(),
        ) {
            Ok(x) => Some(x),
            Err(x) => {
                return Result::Err(format!(
                    "Invalid buffer. Failed to read masking. {x}"
                ))
            }
        }
    } else {
        None
    };

    // todo
    // add metadata, flags, signals, alignments, mods, etc...

    Ok(Sfasta {
        version,
        directory,
        seqlocs,
        index,
        headers,
        ids,
        masking,
        sequenceblocks,
        ..Default::default()
    })
}
