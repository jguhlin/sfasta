use std::sync::Arc;

use libsfasta::{datatypes::StringBlockStoreSeqLocReader, prelude::*};
use polars_core::prelude::*;
use pyo3::prelude::*;
use pyo3_polars::{PyDataFrame, PySeries};
use tokio_stream::StreamExt;

#[pyclass]
struct Sequence
{
    id: Option<String>,
    header: Option<String>,
    sequence: Option<String>,
    scores: Option<String>,
}

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
        let runtime = Arc::new(match threads {
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

    fn __len__(&self) -> usize
    {
        self.runtime.block_on(async move { self.inner.len().await })
    }

    fn __contains__(&self, id: &str) -> bool
    {
        self.runtime.block_on(async move {
            let find = self.inner.find(id).await;
            match find {
                Ok(Some(_)) => true,
                Ok(None) => false,
                Err(_) => false,
            }
        })
    }

    // todo implement __getitem__ to work on seqloc integers

    fn ids(&self) -> PySeries
    {
        let all_ids = self.runtime.block_on(async move {
            let mut all_ids = Vec::new();

            let seqlocs = tokio::spawn(
                Arc::clone(&self.inner.seqlocs.as_ref().unwrap()).stream(),
            );
            let ids = tokio::spawn({
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&self.inner.ids.as_ref().unwrap()),
                    Arc::clone(&self.inner.file_handles),
                )
            });

            let seqlocs = seqlocs.await.unwrap();
            let ids = ids.await.unwrap();

            tokio::pin!(seqlocs);
            tokio::pin!(ids);

            loop {
                let seqloc = match seqlocs.next().await {
                    Some(s) => s,
                    None => break,
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

    fn headers(&self) -> PyDataFrame
    {
        self.runtime.block_on(async move {
            let mut all_ids = Vec::new();
            let mut all_headers = Vec::new();

            let seqlocs = tokio::spawn(
                Arc::clone(&self.inner.seqlocs.as_ref().unwrap()).stream(),
            );
            let ids = tokio::spawn({
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&self.inner.ids.as_ref().unwrap()),
                    Arc::clone(&self.inner.file_handles),
                )
            });

            let headers = tokio::spawn({
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&self.inner.headers.as_ref().unwrap()),
                    Arc::clone(&self.inner.file_handles),
                )
            });

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

    fn seq(&self, id: &str) -> PyResult<PyDataFrame>
    {
        let inner = Arc::clone(&self.inner);
        let seq = self
            .runtime
            .block_on(async move { inner.get_sequence_by_id(id).await });

        match seq {
            Ok(Some(seq)) => {
                let id = match seq.id {
                    Some(id) => String::from_utf8(id.to_vec()).unwrap(),
                    None => "".to_string(),
                };

                let header = match seq.header {
                    Some(header) => String::from_utf8(header.to_vec()).unwrap(),
                    None => "".to_string(),
                };

                let sequence = match seq.sequence {
                    Some(sequence) => {
                        String::from_utf8(sequence.to_vec()).unwrap()
                    }
                    None => "".to_string(),
                };

                let scores = match seq.scores {
                    Some(scores) => String::from_utf8(scores.to_vec()).unwrap(),
                    None => "".to_string(),
                };

                Ok(PyDataFrame(
                    DataFrame::new(vec![
                        Series::new("ID", vec![id]),
                        Series::new("Header", vec![header]),
                        Series::new("Sequence", vec![sequence]),
                        Series::new("Scores", vec![scores]),
                    ])
                    .unwrap(),
                ))
            }
            Ok(None) => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("ID {} not found", id),
            )),
            Err(e) => Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Error: {}", e),
            )),
        }
    }

    fn seqs(&self, query_ids: Vec<String>) -> PyResult<PyDataFrame>
    {
        let mut ids = Vec::with_capacity(query_ids.len());
        let mut headers = Vec::with_capacity(query_ids.len());
        let mut sequences = Vec::with_capacity(query_ids.len());
        let mut scores = Vec::with_capacity(query_ids.len());

        for id in query_ids {
            let inner = Arc::clone(&self.inner);
        let seq = self
            .runtime
            .block_on(async move { inner.get_sequence_by_id(&id).await });

        match seq {
            Ok(Some(seq)) => {
                let id = match seq.id {
                    Some(id) => String::from_utf8(id.to_vec()).unwrap(),
                    None => "".to_string(),
                };

                ids.push(id);

                let header = match seq.header {
                    Some(header) => String::from_utf8(header.to_vec()).unwrap(),
                    None => "".to_string(),
                };

                headers.push(header);

                let sequence = match seq.sequence {
                    Some(sequence) => {
                        String::from_utf8(sequence.to_vec()).unwrap()
                    }
                    None => "".to_string(),
                };

                sequences.push(sequence);

                let scores_ = match seq.scores {
                    Some(scores) => String::from_utf8(scores.to_vec()).unwrap(),
                    None => "".to_string(),
                };

                scores.push(scores_);
            },
            Ok(None) => {
                // Not sure what to do with not found, ignore for now...
            },
            Err(e) => {
                // Not sure what to do with error, ignore for now...
            }
        }

       }

       let id = Series::new("ID", ids);
       let header = Series::new("Header", headers);
       let sequence = Series::new("Sequence", sequences);
       let scores = Series::new("Scores", scores);

       Ok(PyDataFrame(
           DataFrame::new(vec![
               id,
               header,
               sequence,
               scores,
           ])
           .unwrap(),
       ))
    }

    /// Much slower than seqs (which is much more linear)
    /// Weird... maybe tokio and py are not a good combo here?
    /// Or it's something with loading and unloading blocks. dunno

    fn seqs_joinset(&self, query_ids: Vec<String>) -> PyResult<PyDataFrame>
    {
        
        self.runtime.block_on(async move {
            let mut seqs = tokio::task::JoinSet::new();

            for id in query_ids {
                let inner = Arc::clone(&self.inner);
                seqs.spawn(async move {
                    inner.get_sequence_by_id(&id).await
                });
            }

            let mut ids = Vec::new();
            let mut headers = Vec::new();
            let mut sequences = Vec::new();
            let mut scores = Vec::new();

            while let Some(seq) = seqs.join_next().await {
                let seq = seq.unwrap().unwrap();
                match seq {
                    Some(seq) => {
                        let id = match seq.id {
                            Some(id) => String::from_utf8(id.to_vec()).unwrap(),
                            None => "".to_string(),
                        };

                        ids.push(id);

                        let header = match seq.header {
                            Some(header) => {
                                String::from_utf8(header.to_vec()).unwrap()
                            }
                            None => "".to_string(),
                        };
                        headers.push(header);

                        let sequence = match seq.sequence {
                            Some(sequence) => {
                                String::from_utf8(sequence.to_vec()).unwrap()
                            }
                            None => "".to_string(),
                        };

                        sequences.push(sequence);

                        let score = match seq.scores {
                            Some(scores) => {
                                String::from_utf8(scores.to_vec()).unwrap()
                            }
                            None => "".to_string(),
                        };

                        scores.push(score);
                    },
                    None => {
                        // Not sure what to do with not found, ignore for now...
                    }
                }
            }

            let id = Series::new("ID", ids);
            let header = Series::new("Header", headers);
            let sequence = Series::new("Sequence", sequences);
            let scores = Series::new("Scores", scores);

            Ok(PyDataFrame(
                DataFrame::new(vec![
                    id,
                    header,
                    sequence,
                    scores,
                ])
                .unwrap(),
            ))

        }
    )

    }

    fn all_metadata(&self) -> PyDataFrame
    {
        self.runtime.block_on(async move {
            let seqlocs = tokio::spawn(
                Arc::clone(&self.inner.seqlocs.as_ref().unwrap()).stream(),
            );

            let ids = tokio::spawn({
                StringBlockStoreSeqLocReader::new(
                    Arc::clone(&self.inner.ids.as_ref().unwrap()),
                    Arc::clone(&self.inner.file_handles),
                )
            });

            let seqlocs = seqlocs.await.unwrap();
            let ids = ids.await.unwrap();

            tokio::pin!(seqlocs);
            tokio::pin!(ids);

            let mut n = Vec::new();
            let mut all_ids = Vec::new();

            let mut lengths = Vec::new();
            let mut headers = Vec::new();
            let mut masking = Vec::new();
            let mut scores = Vec::new();

            loop {
                let seqloc = match seqlocs.next().await {
                    Some(s) => s,
                    None => break,
                };

                let id = ids.next(seqloc.1.get_ids()).await;
                if let Some(id) = id {
                    // Convert id to String
                    let id = String::from_utf8(id.to_vec()).unwrap();
                    all_ids.push(Some(id));
                } else {
                    all_ids.push(None);
                }

                // Get the sequence
                n.push(seqloc.0);
                let i = self.inner.seqloc_metadata_no_id(&seqloc.1);
                lengths.push(i.length);
                headers.push(i.header);
                masking.push(i.masking);
                scores.push(i.scores);
            }

            let n = Series::new("n", n);
            let ids = Series::new("ID", all_ids);
            let lengths = Series::new("Length", lengths);
            let headers = Series::new("Header", headers);
            let masking = Series::new("Masking", masking);
            let scores = Series::new("Scores", scores);

            let df =
                DataFrame::new(vec![n, ids, lengths, headers, masking, scores])
                    .unwrap();

            PyDataFrame(df)
        })
    }

    /// Get metadata for all sequences without the sequence ids
    /// This is much faster
    // todo implement
    fn metatadata_no_ids(&self) -> PyDataFrame
    {
        self.runtime.block_on(async move {
            let seqlocs = tokio::spawn(
                Arc::clone(&self.inner.seqlocs.as_ref().unwrap()).stream(),
            );

            let seqlocs = seqlocs.await.unwrap();

            tokio::pin!(seqlocs);

            let mut n = Vec::new();
            let mut metadata = Vec::new();

            loop {
                let seqloc = match seqlocs.next().await {
                    Some(s) => s,
                    None => break,
                };

                // Get the sequence
                n.push(seqloc.0);
                let seqloc1 = seqloc.1.clone();
                let myself = Arc::clone(&self.inner);
                metadata.push(self.runtime.spawn(async move {
                    myself.seqloc_metadata(&seqloc1).await
                }));
            }

            // Join all metadata
            // let metadata = futures::future::join_all(metadata).await;
            let mut ids = Vec::new();
            let mut lengths = Vec::new();
            let mut headers = Vec::new();
            let mut masking = Vec::new();
            let mut scores = Vec::new();

            for i in metadata {
                let i = i.await.unwrap();
                ids.push(i.id);
                lengths.push(i.length);
                headers.push(i.header);
                masking.push(i.masking);
                scores.push(i.scores);
            }

            let n = Series::new("n", n);
            let ids = Series::new("ID", ids);
            let lengths = Series::new("Length", lengths);
            let headers = Series::new("Header", headers);
            let masking = Series::new("Masking", masking);
            let scores = Series::new("Scores", scores);

            let df =
                DataFrame::new(vec![n, ids, lengths, headers, masking, scores])
                    .unwrap();

            PyDataFrame(df)
        })
    }

    // todo optimize for getting multiple sequences by ids

    // todo optimize fn for getting multiple sequences by seqloc range
}

/// A Python module implemented in Rust.
#[pymodule]
fn sfasta(m: &Bound<'_, PyModule>) -> PyResult<()>
{
    m.add_class::<Sfasta>()?;
    m.add_class::<Sequence>()?;
    Ok(())
}
