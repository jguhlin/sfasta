#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(unix)]
use std::os::fd::AsRawFd;

use std::sync::Arc;

use dashmap::DashMap;

use tokio::{
    fs::File,
    io::BufReader,
    sync::{Mutex, OwnedMutexGuard},
};

#[derive(Debug)]
pub struct AsyncFileHandleManager
{
    pub file_handles: Arc<DashMap<usize, Arc<Mutex<BufReader<File>>>>>,
    pub file_name: Option<String>,
    pub max_filehandles: u8,
}

impl Default for AsyncFileHandleManager
{
    fn default() -> Self
    {
        AsyncFileHandleManager {
            file_handles: Arc::new(DashMap::new()),
            file_name: None,
            max_filehandles: 8,
        }
    }
}

impl AsyncFileHandleManager
{
    pub async fn get_filehandle(&self) -> OwnedMutexGuard<BufReader<File>>
    {
        loop {
            // Try to get an existing unlocked file handle
            for entry in self.file_handles.iter() {
                let entry = Arc::clone(&entry).try_lock_owned();
                match entry {
                    Ok(entry) => {
                        return entry;
                    }
                    Err(_) => (),
                }
            }

            if self.file_handles.len() < 8 {
                break;
            }

            tokio::time::sleep(tokio::time::Duration::from_millis(1)).await;
        }

        // Otherwise, create one and add it to the list
        let file = tokio::fs::File::open(self.file_name.as_ref().unwrap())
            .await
            .unwrap();

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

        #[cfg(unix)]
        let raw_id = file.as_raw_fd() as usize;

        #[cfg(windows)]
        let raw_id = file.as_raw_handle() as usize;

        #[cfg(not(any(unix, windows)))]
        let raw_id = {
            // Get the largest key from the hashmap and add 1, or 0 if there are
            // no keys
            let mut max = 0;
            for key in self.file_handles.iter() {
                if *key.key() > max {
                    max = *key.key();
                }
            }
            max + 1
        };

        let file_handle = std::sync::Arc::new(Mutex::new(
            tokio::io::BufReader::with_capacity(32 * 1024, file),
        ));

        self.file_handles.insert(raw_id, Arc::clone(&file_handle));

        let fh = file_handle.try_lock_owned().unwrap();

        fh
    }
}
