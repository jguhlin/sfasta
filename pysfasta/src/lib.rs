use std::sync::Arc;

use libsfasta::prelude::*;
use pyo3::prelude::*;

// Async when supported...

#[pyclass]
struct Sfasta
{
    inner: Arc<libsfasta::prelude::Sfasta>,
    runtime: Arc<tokio::runtime::Runtime>,
}

#[pymethods]
impl Sfasta
{
    #[new]
    fn new(path: &str, threads: usize) -> PyResult<Self>
    {
        let runtime = Arc::new(
            match threads {
                0 => tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap(),
                _ => tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(threads)
                    .enable_all()
                    .build()
                    .unwrap(),
            });

        let inner = Arc::new(runtime.block_on(async move {
            open_from_file_async(path).await.unwrap()
        }));

        Ok(Sfasta { inner, runtime })
    }
}

/// A Python module implemented in Rust.
#[pymodule]
fn sfasta(m: &Bound<'_, PyModule>) -> PyResult<()>
{
    m.add_class::<Sfasta>()?;
    Ok(())
}
