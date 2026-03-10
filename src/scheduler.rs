//! Concurrent download scheduler using bounded parallelism.
//!
//! Spawns episode downloads as Tokio tasks, limiting concurrency with a
//! [`Semaphore`]. Aborts all remaining tasks on the first error.

use std::sync::Arc;

use indicatif::MultiProgress;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::downloader;
use crate::error::Error;
use crate::http_client::HttpClient;
use crate::models::Episode;

/// Downloads all given episodes concurrently, up to `max_concurrent` at a time.
///
/// Each episode's metadata is fetched and its files are downloaded inside a
/// spawned Tokio task. A [`Semaphore`] limits how many tasks run in parallel.
/// Progress bars are rendered via the shared [`MultiProgress`] instance.
/// If any task fails, all remaining tasks are aborted and the first error is
/// returned.
///
/// # Errors
///
/// Returns the first error encountered by any download task, or a
/// [`tokio::task::JoinError`] if a spawned task panics.
pub async fn download_all(
    episodes: Vec<Episode>,
    max_concurrent: u8,
    http_client: &Arc<HttpClient>,
    directory: &str,
    multi_progress: &MultiProgress,
    skip_subtitles: bool,
) -> anyhow::Result<()> {
    let semaphore = Arc::new(Semaphore::new(max_concurrent.into()));
    let reqwest_client = http_client.inner().clone();
    let directory = Arc::<str>::from(directory);

    let mut join_set = JoinSet::new();

    for episode in episodes {
        let permit = Arc::clone(&semaphore);
        let mp = multi_progress.clone();
        let client = reqwest_client.clone();
        let http = Arc::clone(http_client);
        let dir = Arc::clone(&directory);

        join_set.spawn(async move {
            let _permit = permit
                .acquire()
                .await
                .map_err(|e| Error::Downloading(e.to_string()))?;

            downloader::fetch_and_download_episode(
                episode,
                &http,
                &mp,
                &client,
                &dir,
                skip_subtitles,
            )
            .await
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
