use crossbeam::utils::Backoff;
use flume::{Receiver, Sender};

use super::*;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::JoinHandle,
};

pub trait Builder<T>
where
    T: Send + Sync + Sized,
{
    fn add(&mut self, input: T) -> Result<Vec<Loc>, &str>;
    fn finalize(&mut self);
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
                            std::thread::sleep(std::time::Duration::from_millis(128));
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
    pub fn join(self) -> Result<Z, std::boxed::Box<(dyn std::any::Any + Send + 'static)>>
    {
        {
            self.shutdown.store(true, Ordering::SeqCst);
        }

        self.worker.unwrap().join()?;

        let builder: Z = Arc::into_inner(self.builder).unwrap().into_inner().unwrap();

        Ok(builder)
    }

    pub fn add(&self, data: T) -> Result<LocMutex, flume::SendError<(LocMutex, T)>>
    {
        let loc = Arc::new((AtomicBool::new(false), Mutex::new(vec![])));
        self.sender.send((Arc::clone(&loc), data))?;
        Ok(loc)
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

        fn finalize(&mut self)
        {
            self.finalized.store(true, Ordering::SeqCst);
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

        let builder = thread_builder.join().unwrap();

        assert!(builder.finalized.load(Ordering::SeqCst));

    }
}
