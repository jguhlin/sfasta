use crossbeam::utils::Backoff;
use flume::{Receiver, Sender};

use super::{super::*, *};

use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
};

pub trait Builder<T>
where
    T: Send + Sync + Sized,
{
    fn add(&mut self, input: T) -> Result<Vec<Loc>, &str>;
    fn finalize(&mut self) -> Result<(), &str>;
}

pub struct ThreadBuilder<T, Z>
where
    T: Send + Sync + Sized,
    Z: Send + Sync + Sized + Builder<T> + 'static,
{
    builder: Arc<Mutex<Z>>,
    shutdown: Arc<AtomicBool>,
    pub sender: Sender<(LocMutex, T)>,
    worker: Option<JoinHandle<()>>,
}

impl<T, Z> ThreadBuilder<T, Z>
where
    T: Send + Sync + 'static + Sized,
    Z: Send + Sync + Sized + Builder<T>,
{
    pub fn new(builder: Z) -> Self
    {
        let builder = Arc::new(Mutex::new(builder));

        let (sender, receiver) = flume::bounded::<(LocMutex, T)>(32);
        let receiver = Arc::new(receiver);
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let receiver_worker = Arc::clone(&receiver);
        let builder_worker = Arc::clone(&builder);
        let shutdown_flag_worker = Arc::clone(&shutdown_flag);

        let handle = std::thread::spawn(move || {
            let receiver = receiver_worker;
            let builder = builder_worker;
            let shutdown_flag = shutdown_flag_worker;

            // Safe to hold the lock the entire time, this is a dedicated thread
            let mut builder = builder.lock().unwrap();

            let backoff = Backoff::new();

            loop {
                for (store_loc, data) in receiver.drain() {
                    let loc = builder.add(data).unwrap();

                    *store_loc.1.lock().unwrap() = loc;
                    store_loc.0.store(true, Ordering::SeqCst);
                }

                if receiver.is_empty() {
                    if shutdown_flag.load(Ordering::SeqCst) {
                        builder.finalize();
                        return;
                    } else if receiver.is_empty() {
                        backoff.snooze();
                        if backoff.is_completed() {
                            std::thread::sleep(
                                std::time::Duration::from_millis(128),
                            );
                            backoff.reset();
                        }
                    }
                }
            }
        });

        Self {
            builder,
            shutdown: shutdown_flag,
            sender,
            worker: Some(handle),
        }
    }

    // Return original builder
    pub fn join(
        self,
    ) -> Result<Z, std::boxed::Box<(dyn std::any::Any + Send + 'static)>>
    {
        {
            self.shutdown.store(true, Ordering::SeqCst);
        }

        self.worker.unwrap().join()?;

        let builder: Z =
            Arc::into_inner(self.builder).unwrap().into_inner().unwrap();

        Ok(builder)
    }

    pub fn add(
        &self,
        data: T,
    ) -> Result<LocMutex, flume::SendError<(LocMutex, T)>>
    {
        let loc = Arc::new((AtomicBool::new(false), Mutex::new(vec![])));
        self.sender.send((Arc::clone(&loc), data))?;
        Ok(loc)
    }
}

/// Specialized threading struct, we have to wait for all
/// data to be available before pushing to the SeqLocsStore
pub struct SeqLocsThreadBuilder
{
    builder: Arc<Mutex<SeqLocsStoreBuilder>>,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,

    // We have to receive and wait on the following:
    // 1. ID Loc
    // 3. Sequence loc (Optional)
    // 2. Masking Loc (Optional)
    // 4. Headers Loc (Optional)
    // 5. Score Loc (Optional)
    // 6. Mods Loc (Optional)
    // 7. Signal Loc (Optional)
    //
    // But as we may store just nanopore signals in the future
    // ... Masking, Seq, should also be optional. Only ID should not be
    // optional

    // First LocMutex is the ID
    pub sender: Sender<(Arc<AtomicU32>, LocMutex, [Option<LocMutex>; 6])>,
}

impl SeqLocsThreadBuilder
{
    pub fn new(builder: SeqLocsStoreBuilder) -> Self
    {
        let builder = Arc::new(Mutex::new(builder));

        let (sender, receiver) = flume::bounded::<(
            Arc<AtomicU32>,
            LocMutex,
            [Option<LocMutex>; 6],
        )>(32);
        let receiver = Arc::new(receiver);
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let receiver_worker = Arc::clone(&receiver);
        let builder_worker = Arc::clone(&builder);
        let shutdown_flag_worker = Arc::clone(&shutdown_flag);

        let handle = std::thread::spawn(move || {
            let receiver = receiver_worker;
            let builder = builder_worker;
            let shutdown_flag = shutdown_flag_worker;

            // Safe to hold the lock the entire time, this is a dedicated thread
            let mut builder = builder.lock().unwrap();

            let backoff = Backoff::new();

            let mut buffer = VecDeque::with_capacity(128);

            loop {
                for (loc_storage, id_loc, locs) in receiver.drain() {
                    buffer.push_back((loc_storage, id_loc, locs));
                }

                while !buffer.is_empty()
                    && buffer[0].1 .0.load(Ordering::SeqCst)
                    && Arc::strong_count(&buffer[0].1) == 1
                    && buffer[0].2.iter().all(|x| {
                        x.is_none()
                            || (x.as_ref().unwrap().0.load(Ordering::SeqCst)
                                && Arc::strong_count(x.as_ref().unwrap()) == 1)
                    })
                {
                    let (loc_storage, id_loc, locs) =
                        buffer.pop_front().unwrap();

                    let id_loc = Arc::into_inner(id_loc)
                        .unwrap()
                        .1
                        .into_inner()
                        .unwrap();
                    let locs: Vec<Vec<Loc>> = locs
                        .into_iter()
                        .map(|x| {
                            if x.is_none() {
                                vec![]
                            } else {
                                let x = x.unwrap();
                                Arc::into_inner(x)
                                    .unwrap()
                                    .1
                                    .into_inner()
                                    .unwrap()
                            }
                        })
                        .collect();

                    let mut seqloc = SeqLoc::new();
                    seqloc.add_locs(
                        &id_loc, &locs[0], &locs[1], &locs[2], &locs[3],
                        &locs[4], &locs[5],
                    );

                    let loc = builder.add_to_index(seqloc);
                    loc_storage.store(loc, Ordering::SeqCst);
                }

                while buffer.is_empty() {
                    if shutdown_flag.load(Ordering::SeqCst) {
                        return;
                    } else if buffer.is_empty() {
                        backoff.snooze();
                        if backoff.is_completed() {
                            backoff.reset();
                        }
                    }
                }
            }
        });

        Self {
            builder,
            shutdown: shutdown_flag,
            worker: Some(handle),
            sender,
        }
    }

    pub fn join(
        self,
    ) -> Result<
        SeqLocsStoreBuilder,
        std::boxed::Box<(dyn std::any::Any + Send + 'static)>,
    >
    {
        {
            self.shutdown.store(true, Ordering::SeqCst);
        }

        self.worker.unwrap().join()?;

        let builder: SeqLocsStoreBuilder =
            Arc::into_inner(self.builder).unwrap().into_inner().unwrap();

        Ok(builder)
    }

    pub fn add(
        &self,
        id: LocMutex,
        locs: [Option<LocMutex>; 6],
    ) -> Result<
        Arc<AtomicU32>,
        flume::SendError<(Arc<AtomicU32>, LocMutex, [Option<LocMutex>; 6])>,
    >
    {
        let loc_storage = Arc::new(AtomicU32::default());
        self.sender.send((Arc::clone(&loc_storage), id, locs))?;
        Ok(loc_storage)
    }
}

#[cfg(test)]
mod tests
{
    use super::*;
    use flume::RecvError;
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread::sleep,
        time::Duration,
    };

    struct TestBuilder
    {
        data: Vec<u8>,
        finalized: AtomicBool,
    }

    impl Builder<Vec<u8>> for TestBuilder
    {
        fn add(&mut self, input: Vec<u8>) -> Result<Vec<Loc>, &str>
        {
            self.data.extend(input);
            Ok(vec![])
        }

        fn finalize(&mut self) -> Result<(), &str>
        {
            self.finalized.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn test_thread_builder()
    {
        let builder = TestBuilder {
            data: vec![],
            finalized: AtomicBool::new(false),
        };

        let mut thread_builder = ThreadBuilder::new(builder);

        println!("Thread builder created");

        let loc = thread_builder.add(vec![1, 2, 3]).unwrap();

        assert!(!loc.0.load(Ordering::SeqCst));

        sleep(Duration::from_millis(128));

        assert!(loc.0.load(Ordering::SeqCst));

        // Test add
        let loc = thread_builder.add(vec![1, 2, 3]).unwrap();
        assert!(!loc.0.load(Ordering::SeqCst));
        sleep(Duration::from_millis(128));
        assert!(loc.0.load(Ordering::SeqCst));

        let builder = thread_builder.join().unwrap();
        assert!(builder.finalized.load(Ordering::SeqCst));


    }
}
