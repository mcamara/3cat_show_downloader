//! Concurrent download scheduler using bounded parallelism.
//!
//! Spawns media downloads as Tokio tasks, limiting concurrency with a
//! [`Semaphore`]. Aborts all remaining tasks on the first error.
//!
//! Renders a single persistent batch-level progress bar at the top of the
//! terminal via [`BatchProgress`] so the user sees `Episode X/N` and an ETA
//! across the whole batch, instead of N spinners stacking as the queue
//! progresses.

use std::sync::Arc;

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::info;

use crate::downloader;
use crate::error::Error;
use crate::models::{DownloadParams, MediaItem};

/// Renders a single batch-level progress bar pinned to the top of the
/// terminal.
///
/// One [`ProgressBar`] is added to the shared [`MultiProgress`] and never
/// finished until every item in the batch has either completed or failed.
/// Each per-item task calls [`Self::inc`] on completion so the bar advances
/// by one tick per episode.
pub(crate) struct BatchProgress {
    bar: ProgressBar,
}

impl BatchProgress {
    /// Creates a new batch progress bar bound to the supplied
    /// [`MultiProgress`]. The bar is rendered as
    /// `Batch [████░░░░] 42% (10/24) ETA 03:12`.
    pub(crate) fn new(mp: &MultiProgress, total: u64) -> Self {
        let bar = mp.add(ProgressBar::new(total));
        bar.set_style(
            ProgressStyle::with_template(
                "{prefix:.bold} [{bar:30.cyan/blue}] {percent}% ({pos}/{len}) ETA {eta}",
            )
            .expect("static progress template must compile")
            .progress_chars("██░"),
        );
        bar.set_prefix("Batch".to_string());
        // No `enable_steady_tick` here: on terminals that don't support
        // ANSI cursor movement (legacy PowerShell 5.1 host, redirected
        // output, CI runners) the periodic redraw would emit a fresh line
        // every tick instead of overwriting the previous one. Indicatif
        // repaints on state changes only — `inc()`, `finish_and_clear()` —
        // which is the cleanest behaviour everywhere.
        Self { bar }
    }

    /// Advances the bar by one completed item.
    pub(crate) fn inc(&self) {
        self.bar.inc(1);
    }

    /// Finalises the bar with a summary message and removes it from the
    /// renderer so the prompt returns to a clean state.
    pub(crate) fn finish(&self, total: u64) {
        self.bar.set_message(format!("{total}/{total} done"));
        self.bar.finish_and_clear();
    }
}

/// Downloads all given media items concurrently, up to
/// [`DownloadParams::concurrent_downloads`] at a time.
///
/// Each item's metadata is fetched and its files are downloaded inside a
/// spawned Tokio task. A [`Semaphore`] limits how many tasks run in parallel.
/// Progress bars are rendered via the [`DownloadParams::multi_progress`]
/// instance. If any task fails, all remaining tasks are aborted and the first
/// error is returned.
///
/// Per-task subtitle failures in tolerant mode are logged as warnings
/// inside [`downloader::fetch_and_download_media`] and do not propagate
/// here, so the batch continues. Pass `--strict-subtitles` to revert to
/// the fail-fast behaviour (any subtitle error aborts the whole batch).
///
/// # Errors
///
/// Returns the first error encountered by any download task, or a
/// [`tokio::task::JoinError`] if a spawned task panics.
pub async fn download_all(items: Vec<MediaItem>, params: &DownloadParams) -> anyhow::Result<()> {
    let total = items.len() as u64;
    let batch = Arc::new(BatchProgress::new(&params.multi_progress, total));
    let semaphore = Arc::new(Semaphore::new(params.concurrent_downloads.into()));

    let mut join_set = JoinSet::new();

    for item in items {
        let permit = Arc::clone(&semaphore);
        let task_params = params.clone();
        let batch = Arc::clone(&batch);

        join_set.spawn(async move {
            let _permit = permit
                .acquire()
                .await
                .map_err(|e| Error::Downloading(e.to_string()))?;

            let result = downloader::fetch_and_download_media(item, &task_params).await;
            batch.inc();
            result
        });
    }

    while let Some(result) = join_set.join_next().await {
        match result {
            Ok(Ok(_subtitle_failed)) => {}
            Ok(Err(e)) => {
                join_set.abort_all();
                batch.finish(total);
                return Err(e.into());
            }
            Err(join_err) => {
                join_set.abort_all();
                if join_err.is_cancelled() {
                    continue;
                }
                batch.finish(total);
                return Err(join_err.into());
            }
        }
    }

    batch.finish(total);
    info!("Batch finished: {total}/{total} episodes processed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_construct_batch_progress_with_zero_total() {
        let mp = MultiProgress::new();
        let bp = BatchProgress::new(&mp, 0);
        bp.inc();
        bp.finish(0);
    }
}
