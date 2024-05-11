use std::io::{BufReader, Read, Seek};
use std::sync::Arc;

use tokio::io::{AsyncBufRead, AsyncRead, AsyncReadExt, AsyncSeek, AsyncSeekExt};

use libfractaltree::FractalTreeDisk;
use crate::{datatypes::*, formats::*};

#[cfg(unix)]
use std::os::fd::AsRawFd;
// todo add the hints back in for random access

const SFASTA_MARKER: &[u8; 6] = b"sfasta";
const FULL_HEADER_SIZE: usize =
    6 + std::mem::size_of::<u64>() + std::mem::size_of::<DirectoryOnDisk>();

/// Open a SFASTA file from a filename.
///
/// Multiple threads are used to read the file.
#[tokio::main(flavor = "multi_thread")]
pub async fn open_from_file_async<'sfa>(file: &str) -> Result<Sfasta<'sfa>, String>
{
    log::debug!("Opening file: {file}");
    let bincode_config_fixed = crate::BINCODE_CONFIG
        .with_fixed_int_encoding()
        .with_limit::<{ 2 * 1024 * 1024 }>();

    // Create shareable string for the filename
    let file = file.to_string();
    let file = std::sync::Arc::new(file);

    let mut in_buf = match std::fs::File::open(&(*file)) {
        Ok(x) => x,
        Err(x) => return Result::Err(format!("Failed to open file: {x}")),
    };

    log::debug!("Header Size: {FULL_HEADER_SIZE}");

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
            format!("Invalid buffer. SFASTA marker is missing. Found: {} Expected: {}",
                std::str::from_utf8(&sfasta_header[0..6]).unwrap(),
                std::str::from_utf8(SFASTA_MARKER).unwrap())
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

    drop(in_buf);

    // Note: dropped the length check, it was mostly for fuzzing, but
    // should be fine without it if we make sure to check everything
    // as we go

    let directory: Directory = directory.into();

    // Preload the seqlocs and index

    // Now that the main checks are out of the way, switch to async mode

    // Put this into an async block
    let file_name = Arc::clone(&file);
    let seqlocs = tokio::spawn(async move {
        log::debug!("SeqLocs");
        let in_buf = tokio::fs::File::open(file_name.as_str()).await.unwrap();
        let mut in_buf = tokio::io::BufReader::new(in_buf);
        let seqlocs: Option<SeqLocsStore> = match SeqLocsStore::from_existing_async(
            directory.seqlocs_loc.unwrap().get(),
            &mut in_buf,
        ).await {
            Ok(x) => Some(x),
            Err(x) => {
                return Result::Err(format!(
                    "Invalid buffer. Failed to read seqlocs. {x}"
                ))
            }
        };
        log::debug!("Got SeqLocs");
        Ok(seqlocs)
    });
    
    let file_name = Arc::clone(&file);
    let index = tokio::spawn(async move {
        let in_buf = tokio::fs::File::open(file_name.as_str()).await.unwrap();
        let mut in_buf = tokio::io::BufReader::new(in_buf);
        let index: Option<FractalTreeDisk<u32, u32>> = if directory.index_loc.is_some() {
            match FractalTreeDisk::from_buffer_async(
                &mut in_buf,
                directory.index_loc.unwrap().get(),
            ).await {
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
        Ok(index)
    });
    
    let file_name = Arc::clone(&file);
    let sequenceblocks = tokio::spawn(async move {
        let in_buf = tokio::fs::File::open(file_name.as_str()).await.unwrap();
        let mut in_buf = tokio::io::BufReader::new(in_buf);
        let sequenceblocks = if directory.sequences_loc.is_some() {
            match BytesBlockStore::from_buffer_async(
                &mut in_buf,
                directory.sequences_loc.unwrap().get(),
            ).await {
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
        Ok(sequenceblocks)
    });

    let file_name = Arc::clone(&file);
    let ids = tokio::spawn(async move {
        let in_buf = tokio::fs::File::open(file_name.as_str()).await.unwrap();
        let mut in_buf = tokio::io::BufReader::new(in_buf);
        let ids = if directory.ids_loc.is_some() {
            match StringBlockStore::from_buffer_async(
                &mut in_buf,
                directory.ids_loc.unwrap().get(),
            ).await {
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
        Ok(ids)
    });

    let file_name = Arc::clone(&file);
    let headers = tokio::spawn(async move {
        let in_buf = tokio::fs::File::open(file_name.as_str()).await.unwrap();
        let mut in_buf = tokio::io::BufReader::new(in_buf);
        let headers = if directory.headers_loc.is_some() {
            match StringBlockStore::from_buffer_async(
                &mut in_buf,
                directory.headers_loc.unwrap().get(),
            ).await {
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
        Ok(headers)
    });

    let file_name = Arc::clone(&file);
    let masking = tokio::spawn(async move {
        let in_buf = tokio::fs::File::open(file_name.as_str()).await.unwrap();
        let mut in_buf = tokio::io::BufReader::new(in_buf);
        let masking = if directory.masking_loc.is_some() {
            match Masking::from_buffer_async(
                &mut in_buf,
                directory.masking_loc.unwrap().get(),
            ).await {
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
        Ok(masking)
    });

    // todo
    // add metadata, flags, signals, alignments, mods, etc...

    let (seqlocs, index, headers, ids, masking, sequenceblocks) = tokio::try_join!(
        seqlocs,
        index,
        headers,
        ids,
        masking,
        sequenceblocks
    ).expect("Failed to join async tasks");

    let seqlocs = seqlocs.unwrap();
    let index = index.unwrap();
    let headers = headers.unwrap();
    let ids = ids.unwrap();
    let masking = masking.unwrap();
    let sequenceblocks = sequenceblocks.unwrap();

    Ok(Sfasta {
        version,
        directory,
        seqlocs,
        index,
        headers,
        ids,
        masking,
        sequenceblocks,
        file: Some(file.to_string()),
        ..Default::default()
    })
}

// Bincode hack
// Keep reading the buffer until we can do a proper decode
// This is a hack to work around the fact that bincode doesn't support
// tokio
pub(crate) async fn bincode_decode_from_buffer_async<T>(
    in_buf: &mut tokio::io::BufReader<tokio::fs::File>,
    bincode_config: bincode::config::Configuration,
) -> Result<T, String>
where
    T: bincode::Decode,
{
    let mut buf = vec![0; 8 * 1024];
    let mut bytes_read = in_buf.read(&mut buf).await.unwrap();
    // buf.shrink_to(bytes_read);

    loop {
        match bincode::decode_from_slice(&buf, bincode_config) {
            Ok(x) => {
                return Ok(x.0);
            }
            Err(_) => {
                let orig_length = buf.len();
                let doubled = buf.len() * 2;

                buf.resize(doubled, 0);

                bytes_read = in_buf.read(&mut buf[orig_length..]).await.unwrap();                
                // buf.shrink_to(bytes_read);

                if doubled > 16 * 1024 * 1024 {
                    return Result::Err("Failed to decode bincode".to_string());
                }
            }
        }
    }
}

// todo curry size_hint as const
pub(crate) async fn bincode_decode_from_buffer_async_with_size_hint<T, C>(
    in_buf: &mut tokio::io::BufReader<tokio::fs::File>,
    bincode_config: C,
    size_hint: usize,
) -> Result<T, String>
where
    T: bincode::Decode,
    C: bincode::config::Config,
{
    let mut buf = vec![0; size_hint];
    let mut bytes_read = in_buf.read(&mut buf).await.unwrap();

    loop {
        match bincode::decode_from_slice(&buf, bincode_config) {
            Ok(x) => {
                return Ok(x.0);
            }
            Err(_) => {
                let orig_length = buf.len();
                let doubled = buf.len() * 2;

                buf.resize(doubled, 0);

                bytes_read = in_buf.read(&mut buf[orig_length..]).await.unwrap();

                if doubled > 16 * 1024 * 1024 {
                    return Result::Err("Failed to decode bincode".to_string());
                }
            }
        }
    }
}
