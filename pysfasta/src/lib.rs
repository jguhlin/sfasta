use std::sync::Arc;

use libsfasta::prelude::*;
use pyo3::prelude::*;
use pyo3_polars::{PySeries, PyDataFrame};
use libsfasta::datatypes::StringBlockStoreSeqLocReader;
use tokio_stream::StreamExt;
use polars_core::prelude::*;

// Pyo3 async fn when stabilised

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

    fn ids(&self) -> PySeries
    {
        let all_ids = self.runtime.block_on(async move {

            let mut all_ids = Vec::new();

            let seqlocs = tokio::spawn(Arc::clone(&self.inner.seqlocs.as_ref().unwrap()).stream());
            let ids = tokio::spawn( {
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&self.inner.ids.as_ref().unwrap()),
                    Arc::clone(&self.inner.file_handles),
            )});

            let seqlocs = seqlocs.await.unwrap();
            let ids = ids.await.unwrap();

            tokio::pin!(seqlocs);
            tokio::pin!(ids);

            loop {
                let seqloc = match seqlocs.next().await {
                    Some(s) => s,
                    None => {
                        break
                    },
                };

                // Get the sequence

                let seqloc = Arc::new(seqloc);

                let id = ids.next(seqloc.1.get_ids()).await;
                if let Some(id) = id {
                    // Convert id to String
                    let id = String::from_utf8(id.to_vec()).unwrap();
                    all_ids.push(id);
                }
            }

            all_ids
        });

        // let all_ids: Series = all_ids.into_iter().collect();
        let all_ids = Series::new("ID", all_ids);

        PySeries(all_ids)
    }

    fn headers(&self) -> PyDataFrame {
        self.runtime.block_on(async move {

            let mut all_ids = Vec::new();
            let mut all_headers = Vec::new();

            let seqlocs = tokio::spawn(Arc::clone(&self.inner.seqlocs.as_ref().unwrap()).stream());
            let ids = tokio::spawn( {
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&self.inner.ids.as_ref().unwrap()),
                    Arc::clone(&self.inner.file_handles),
            )});

            let headers = tokio::spawn( {
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&self.inner.headers.as_ref().unwrap()),
                    Arc::clone(&self.inner.file_handles),
            )});

            let seqlocs = seqlocs.await.unwrap();
            let ids = ids.await.unwrap();
            let headers = headers.await.unwrap();

            tokio::pin!(seqlocs);
            tokio::pin!(ids);
            tokio::pin!(headers);

            loop {
                let seqloc = match seqlocs.next().await {
                    Some(s) => s,
                    None => break,
                };

                // Get the sequence

                let seqloc = Arc::new(seqloc);

                let id = ids.next(seqloc.1.get_ids()).await;
                let header = headers.next(seqloc.1.get_headers()).await;

                if let Some(id) = id {
                    // Convert id to String
                    let id = String::from_utf8(id.to_vec()).unwrap();
                    all_ids.push(id);
                } else {
                    all_ids.push("".to_string());
                }

                if let Some(header) = header {
                    // Convert header to String
                    let header = String::from_utf8(header.to_vec()).unwrap();
                    all_headers.push(header);
                } else {
                    all_headers.push("".to_string());
                }

            }

            let all_ids = Series::new("ID", all_ids);
            let all_headers = Series::new("Header", all_headers);

            let df = DataFrame::new(vec![all_ids, all_headers]).unwrap();
            PyDataFrame(df)
        })
    }
}

/// A Python module implemented in Rust.
#[pymodule]
fn sfasta(m: &Bound<'_, PyModule>) -> PyResult<()>
{
    m.add_class::<Sfasta>()?;
    Ok(())
}
