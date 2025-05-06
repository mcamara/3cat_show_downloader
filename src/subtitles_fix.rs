use crate::models::Episode;
use anyhow::Result;
use std::path::PathBuf;
use glob::GlobError;
use tracing::error; 

pub fn fix_subtitles(episode: &Episode, directory: &str) -> Result<()> {
    // Find all files with "name.<language>.vtt" in the directory
    let pattern = format!("{}.*.vtt", episode.filename());
    // Explicitly type paths for clarity, glob::glob(...)? returns an iterator of Result<PathBuf, GlobError>
    let paths: Vec<Result<PathBuf, GlobError>> = glob::glob(&format!("{}/{}", directory, pattern))?
        .collect();

    if paths.is_empty() {
        println!("No subtitle files found for episode {}", episode.filename());
        return Ok(());
    }

    for path_result in &paths { 
        let path = match path_result {
            Ok(path) => path,
            Err(e) => {
                error!(error = %e, "Error finding subtitle file: {}", e);
                continue;
            }
        };

        println!("Found subtitle file: {:?}", path);
    }

    Ok(())
}
