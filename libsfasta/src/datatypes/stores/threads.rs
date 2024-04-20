use super::*;
use flume::{Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
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
    wakeup: Wakeup,
    pub sender: Sender<(LocMutex, T)>,
    worker: Option<JoinHandle<()>>,
}

impl<T> ThreadBuilder<T> 
where T: Send + Sync + 'static {
    pub fn new(builder: impl Builder<T> + Send + Sync + 'static) -> Self {

        let builder = Arc::new(Mutex::new(builder));

        let (sender, receiver) = flume::bounded::<(LocMutex, T)>(32);
        let receiver = Arc::new(receiver);
        let wakeup = Arc::new((Mutex::new((false, false)), Condvar::new()));

        let receiver_worker = Arc::clone(&receiver);
        let builder_worker = Arc::clone(&
            builder);
        let wakeup_worker = Arc::clone(&wakeup);

        let handle = std::thread::spawn(move || {
            let receiver = receiver_worker;
            let builder = builder_worker;
            let wakeup = wakeup_worker;

            // Safe to hold the lock the entire time, this is a dedicated thread
            let mut builder = builder.lock().unwrap();

            loop {
                for (store_loc, data) in receiver.drain() {
                    let loc = builder.add(data).unwrap();

                    let mut store_loc = store_loc.lock().unwrap();
                    store_loc.1 = loc;
                    store_loc.0 = true;
                }

                if receiver.is_empty() {
                    let (guards, condvar) = &*wakeup;
                    let guard = guards.lock().unwrap();

                    if guard.1 && receiver.is_empty() && !guard.0 {
                        builder.finalize();
                        return;
                    } else if !guard.0 || !receiver.is_empty() {
                        // Lock until we have data
                        // We are ok with spurious wakeups, as we don't expect to sleep often
                        let _guard = condvar.wait(guard).expect("Failed to wait on condvar");
                    }
                }
            }
        });

        Self {
            builder,
            wakeup,
            sender,
            worker: Some(handle),
        }
    }

    pub fn join(self) -> JoinHandle<()> {
        self.worker.unwrap()
    }

}