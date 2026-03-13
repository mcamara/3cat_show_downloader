//! Concurrent download scheduler using bounded parallelism.
//!
//! Spawns media downloads as Tokio tasks, limiting concurrency with a
//! [`Semaphore`]. Aborts all remaining tasks on the first error.

use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::downloader;
use crate::error::Error;
use crate::models::{DownloadParams, MediaItem};

/// Downloads all given media items concurrently, up to
/// [`DownloadParams::concurrent_downloads`] at a time.
///
/// Each item's metadata is fetched and its files are downloaded inside a
/// spawned Tokio task. A [`Semaphore`] limits how many tasks run in parallel.
/// Progress bars are rendered via the [`DownloadParams::multi_progress`]
/// instance. If any task fails, all remaining tasks are aborted and the first
/// error is returned.
///
/// # Errors
///
/// Returns the first error encountered by any download task, or a
/// [`tokio::task::JoinError`] if a spawned task panics.
pub async fn download_all(items: Vec<MediaItem>, params: &DownloadParams) -> anyhow::Result<()> {
    let semaphore = Arc::new(Semaphore::new(params.concurrent_downloads.into()));

    let mut join_set = JoinSet::new();

    for item in items {
        let permit = Arc::clone(&semaphore);
        let task_params = params.clone();

        join_set.spawn(async move {
            let _permit = permit
                .acquire()
                .await
                .map_err(|e| Error::Downloading(e.to_string()))?;

            downloader::fetch_and_download_media(item, &task_params).await
        });
    }

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                join_set.abort_all();
                return Err(e.into());
            }
            Err(join_err) => {
                join_set.abort_all();
                if join_err.is_cancelled() {
                    continue;
                }
                return Err(join_err.into());
            }
        }
    }

    Ok(())
}
