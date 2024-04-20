use flume::{Receiver, Sender};
use crossbeam::utils::Backoff;

use super::*;
use std::sync::{Arc, Condvar, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread::JoinHandle;

pub(crate) trait Builder<T>
where
    T: Send + Sync,
{
    fn add(&mut self, input: T) -> Result<Vec<Loc>, &str>;
    fn finalize(&mut self);
}

pub(crate) struct ThreadBuilder<T>
where
    T: Send + Sync,
{
    builder: Arc<Mutex<dyn Builder<T> + Send + Sync>>,
    shutdown: Arc<AtomicBool>,
    pub sender: Sender<(LocMutex, T)>,
    worker: Option<JoinHandle<()>>,
}

impl<T> ThreadBuilder<T> 
where T: Send + Sync + 'static {
    pub fn new(builder: impl Builder<T> + Send + Sync + 'static) -> Self {

        let builder = Arc::new(Mutex::new(builder));

        let (sender, receiver) = flume::bounded::<(LocMutex, T)>(32);
        let receiver = Arc::new(receiver);
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let receiver_worker = Arc::clone(&receiver);
        let builder_worker = Arc::clone(&
            builder);

        let shutdown_flag_worker = Arc::clone(&shutdown_flag);



        let handle = std::thread::spawn(move || {
            let receiver = receiver_worker;
            let builder = builder_worker;
            let shutdown_flag = shutdown_flag_worker;

            // Safe to hold the lock the entire time, this is a dedicated thread
            let mut builder = builder.lock().unwrap();

            println!("Thread spawned");

            let backoff = Backoff::new();

            loop {
                for (store_loc, data) in receiver.drain() {
                    let loc = builder.add(data).unwrap();

                    let mut store_loc = store_loc.lock().unwrap();
                    store_loc.1 = loc;
                    store_loc.0 = true;
                }

                if receiver.is_empty() {
                    if shutdown_flag.load(Ordering::SeqCst) {
                        println!("Shutting down");
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

    pub fn join(self) -> Result<(), std::boxed::Box<(dyn std::any::Any + Send + 'static)>> {
        {
            self.shutdown.store(true, Ordering::SeqCst);
            
        }
        
        self.worker.unwrap().join()
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use flume::RecvError;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread::sleep;
    use std::time::Duration;

    struct TestBuilder {
        data: Vec<u8>,
        finalized: AtomicBool,
    }

    impl Builder<Vec<u8>> for TestBuilder {
        fn add(&mut self, input: Vec<u8>) -> Result<Vec<Loc>, &str> {
            self.data.extend(input);
            Ok(vec![])
        }

        fn finalize(&mut self) {
            self.finalized.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn test_thread_builder() {
        let builder = TestBuilder {
            data: vec![],
            finalized: AtomicBool::new(false),
        };

        let mut thread_builder = ThreadBuilder::new(builder);

        println!("Thread builder created");

        let data = vec![1, 2, 3, 4, 5];
        let loc = Arc::new(Mutex::new((false, vec![])));
        thread_builder.sender.send((Arc::clone(&loc), data)).unwrap();

        sleep(Duration::from_millis(100));

        let loc_return = loc.lock().unwrap();
        assert_eq!(loc_return.0, true);
        assert_eq!(loc_return.1, vec![]);

        let data = vec![6, 7, 8, 9, 10];
        let loc = Arc::new(Mutex::new((false, vec![])));
        thread_builder.sender.send((Arc::clone(&loc), data)).unwrap();

        sleep(Duration::from_millis(100));

        let loc_return = loc.lock().unwrap();
        assert_eq!(loc_return.0, true);
        assert_eq!(loc_return.1, vec![]);

        let data = vec![11, 12, 13, 14, 15];
        let loc = Arc::new(Mutex::new((false, vec![])));
        thread_builder.sender.send((Arc::clone(&loc), data)).unwrap();

        sleep(Duration::from_millis(100));

        let loc_return = loc.lock().unwrap();
        assert_eq!(loc_return.0, true);
        assert_eq!(loc_return.1, vec![]);

        let data = vec![16, 17, 18, 19, 20];
        let loc = Arc::new(Mutex::new((false, vec![])));
        thread_builder.sender.send((Arc::clone(&loc), data)).unwrap();

        sleep(Duration::from_millis(100));

        let loc = loc.lock().unwrap();
        assert_eq!(loc.0, true);
        assert_eq!(loc.1, vec![]);

        thread_builder.join().expect("Failed to join thread");

    }
}
