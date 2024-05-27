use std::{
    io::{BufReader, Read, Seek},
    sync::Arc,
};

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncSeek,
    AsyncSeekExt, SeekFrom
};

use tokio::task::spawn_blocking;

use crate::{datatypes::*, formats::*};
use libfractaltree::FractalTreeDiskAsync;

use libfilehandlemanager::AsyncFileHandleManager;

#[cfg(unix)]
use std::os::fd::AsRawFd;

// note: this is SLOWER than the sequential version
// 22ms vs 15msg
// but 14ms using single thread mode
// UPDATE: Down to 14-15ms by loading the filehandles earlier...
const SFASTA_MARKER: &[u8; 6] = b"sfasta";
const FULL_HEADER_SIZE: usize =
    6 + std::mem::size_of::<u64>() + std::mem::size_of::<DirectoryOnDisk>();

/// Open a SFASTA file from a filename.
///
/// Multiple threads are used to read the file.
#[tracing::instrument]
pub async fn open_from_file_async<'sfa>(
    file: &str,
) -> Result<Sfasta<'sfa>, String>
{
    // Create shareable string for the filename
    let file = file.to_string();
    let file = std::sync::Arc::new(file);

    // Open up 6 file handles (one per task)
    let file_name2 = Arc::clone(&file);
    let file_handles = tokio::spawn(async move {
        let mut file_handles = Vec::new();
        for _ in 0..6 {
            let file =
                tokio::fs::File::open(file_name2.as_str()).await.unwrap();

            #[cfg(unix)]
            {
                nix::fcntl::posix_fadvise(
                    file.as_raw_fd(),
                    0,
                    0,
                    nix::fcntl::PosixFadviseAdvice::POSIX_FADV_RANDOM,
                )
                .expect("Fadvise Failed");
            }

            let file = tokio::io::BufReader::with_capacity(64 * 1024, file);

            file_handles.push(file);
        }

        file_handles
    });

    log::debug!("Opening file: {file}");
    let bincode_config_fixed = crate::BINCODE_CONFIG
        .with_fixed_int_encoding()
        .with_limit::<{ 512 * 1024 }>();

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
        return Result::Err(format!(
            "Invalid buffer. SFASTA marker is missing. Found: {} Expected: {}",
            std::str::from_utf8(&sfasta_header[0..6]).unwrap(),
            std::str::from_utf8(SFASTA_MARKER).unwrap()
        ));
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

    let mut file_handles = file_handles.await.unwrap();

    // Preload the seqlocs and index

    // Now that the main checks are out of the way, switch to async mode
    let filename = file.to_string();
    let seqlocs = tokio::spawn(async move {
        let seqlocs = match SeqLocsStore::from_existing(
            filename,
            directory.seqlocs_loc.unwrap().get(),
        )
        .await
        {
            Ok(x) => Some(Arc::new(x)),
            Err(x) => {
                return Result::Err(format!(
                    "Invalid buffer. Failed to read seqlocs. {x}"
                ))
            }
        };
        Ok(seqlocs)
    });

    let filename = file.to_string();
    let index = tokio::spawn(async move {
        let index: Option<Arc<FractalTreeDiskAsync<u32, u32>>> =
            if directory.index_loc.is_some() {
                match FractalTreeDiskAsync::from_buffer(
                    filename,
                    directory.index_loc.unwrap().get(),
                )
                .await
                {
                    Ok(x) => Some(Arc::new(x)),
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

    let filename = file.to_string();
    let fh = file_handles.pop().unwrap();
    let sequenceblocks = tokio::spawn(async move {
        let sequenceblocks = if directory.sequences_loc.is_some() {
            let mut in_buf = fh;
            match BytesBlockStore::from_buffer(
                &mut in_buf,
                filename,
                directory.sequences_loc.unwrap().get(),
            )
            .await
            {
                Ok(x) => Some(Arc::new(x)),
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

    let filename = file.to_string();
    let fh = file_handles.pop().unwrap();
    let ids = tokio::spawn(async move {
        let ids = if directory.ids_loc.is_some() {
            let mut in_buf = fh;
            match StringBlockStore::from_buffer(
                &mut in_buf,
                filename,
                directory.ids_loc.unwrap().get(),
            )
            .await
            {
                Ok(x) => Some(Arc::new(x)),
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

    let filename = file.to_string();
    let fh = file_handles.pop().unwrap();
    let headers = tokio::spawn(async move {
        let headers = if directory.headers_loc.is_some() {
            let mut in_buf = fh;
            match StringBlockStore::from_buffer(
                &mut in_buf,
                filename,
                directory.headers_loc.unwrap().get(),
            )
            .await
            {
                Ok(x) => Some(Arc::new(x)),
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

    let filename = file.to_string();
    let fh = file_handles.pop().unwrap();
    let masking = tokio::spawn(async move {
        let masking = if directory.masking_loc.is_some() {
            let mut in_buf = fh;
            match Masking::from_buffer(
                &mut in_buf,
                filename,
                directory.masking_loc.unwrap().get(),
            )
            .await
            {
                Ok(x) => Some(Arc::new(x)),
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

    // Let's store some more filehandles for the future
    let file_name2 = Arc::clone(&file);
    let file_handles = tokio::spawn(async move {
        let mut file_handles = Vec::new();
        for _ in 0..4 {
            let file =
                tokio::fs::File::open(file_name2.as_str()).await.unwrap();

            #[cfg(unix)]
            {
                nix::fcntl::posix_fadvise(
                    file.as_raw_fd(),
                    0,
                    0,
                    nix::fcntl::PosixFadviseAdvice::POSIX_FADV_RANDOM,
                )
                .expect("Fadvise Failed");
            }

            let file = tokio::io::BufReader::with_capacity(128 * 1024, file);

            file_handles
                .push(std::sync::Arc::new(tokio::sync::Mutex::new(file)));
        }

        file_handles
    });

    log::trace!("Waiting for async tasks to finish");

    let (seqlocs, index, headers, ids, masking, sequenceblocks) =
        tokio::try_join!(seqlocs, index, headers, ids, masking, sequenceblocks)
            .expect("Failed to join async tasks");

    log::trace!("Finished waiting for async tasks to finish");

    let seqlocs = seqlocs.unwrap();
    let index = index.unwrap();
    let headers = headers.unwrap();
    let ids = ids.unwrap();
    let masking = masking.unwrap();
    let sequenceblocks = sequenceblocks.unwrap();

    log::trace!("Finished reading file: {file}");

    Ok(Sfasta {
        version,
        directory,
        seqlocs,
        index,
        headers,
        ids,
        masking,
        sequences: sequenceblocks,
        file: Some(file.to_string()),
        file_handles: Arc::new(AsyncFileHandleManager {
            file_name: Some(file.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    })

    // Ok(Sfasta::default())
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
    // Try to read without creating a new buffer - was not faster

    let mut buf = vec![0; in_buf.buffer().len() + 16 * 1024];
    in_buf.read(&mut buf).await.unwrap();
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

                in_buf.read(&mut buf[orig_length..]).await.unwrap();

                if doubled > 256 * 1024 * 1024 {
                    return Result::Err("Failed to decode bincode".to_string());
                }
            }
        }
    }
}

pub(crate) async fn bincode_decode_from_buffer_async_with_size_hint<
    const SIZE_HINT: usize,
    T,
    C,
>(
    in_buf: &mut tokio::io::BufReader<tokio::fs::File>,
    bincode_config: C,
) -> Result<T, String>
where
    T: bincode::Decode,
    C: bincode::config::Config,
{
    let start_pos = in_buf.stream_position().await.unwrap();

    let current_buffer = in_buf.fill_buf().await.unwrap();
    if current_buffer.len() == 0 {
        return Result::Err("Failed to read buffer".to_string());
    }

    // If we can get it direct from the buffer, do so...
    match bincode::decode_from_slice(&current_buffer, bincode_config) {
        Ok((_, 0)) => (),
        Ok((x, size)) => {
            in_buf.consume(size);
            in_buf
                .seek(SeekFrom::Start(start_pos + size as u64))
                .await
                .unwrap();

            return Ok(x);
        }
        Err(_) => (),
    };

    let mut buf = vec![0; SIZE_HINT];
    match in_buf.read(&mut buf).await {
        Ok(_) => (),
        Err(_) => return Result::Err("Failed to read buffer".to_string()),
    }

    loop {
        match bincode::decode_from_slice(&buf, bincode_config) {
            Ok((_, 0)) => (),
            Ok((x, size)) => {
                in_buf
                    .seek(SeekFrom::Start(start_pos + size as u64))
                    .await
                    .unwrap();

                // todo read from borrowed buffer, then advance that far, rather
                // than seeking back see: https://docs.rs/tokio/latest/tokio/io/trait.AsyncBufReadExt.html#method.fill_buf
                // fill buf and consume

                return Ok(x);
            }
            Err(_) => (),
        };

        let orig_length = buf.len();
        let doubled = buf.len() * 2;

        buf.resize(doubled, 0);

        match in_buf.read(&mut buf[orig_length..]).await {
            Ok(_) => (),
            Err(_) => return Result::Err("Failed to read buffer".to_string()),
        }

        if doubled > 16 * 1024 * 1024 {
            return Result::Err("Failed to decode bincode".to_string());
        }
    }
}

pub(crate) async fn bincode_decode_from_buffer_async_with_size_hint_nc<T, C>(
    in_buf: &mut tokio::io::BufReader<tokio::fs::File>,
    bincode_config: C,
    size_hint: usize,
) -> Result<T, String>
where
    T: bincode::Decode,
    C: bincode::config::Config,
{
    let mut buf = vec![0; size_hint];
    in_buf.read(&mut buf).await.unwrap();

    loop {
        match bincode::decode_from_slice(&buf, bincode_config) {
            Ok(x) => {
                return Ok(x.0);
            }
            Err(_) => {
                let orig_length = buf.len();
                let doubled = buf.len() * 2;

                buf.resize(doubled, 0);

                in_buf.read(&mut buf[orig_length..]).await.unwrap();

                if doubled > 256 * 1024 * 1024 {
                    return Result::Err(
                        "Failed to decode bincode - Max size reached"
                            .to_string(),
                    );
                }
            }
        }
    }
}
