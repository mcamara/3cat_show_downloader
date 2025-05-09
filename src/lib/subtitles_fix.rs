use crate::{
    error::Error,
    models::{Episode, Subtitle},
};
use anyhow::Result;
use glob::GlobError;
use once_cell::sync::Lazy;
use regex::Regex;
use std::{
    io::{BufRead, BufReader},
    path::PathBuf,
};
use tracing::{debug, error, info, warn};

static REGEX_SUBTITLE_FIX: Lazy<Regex> = Lazy::new(|| Regex::new(r"^Region:").unwrap());

pub fn fix_subtitles(episode: &Episode, directory: &str) -> Result<()> {
    let paths = get_subtitles_pats(episode, directory)?;

    let subtitles: Vec<Subtitle> = paths
        .into_iter()
        .filter_map(|path| match clean_and_build_subtitle(path) {
            Ok(subtitle) => Some(subtitle),
            Err(e) => {
                warn!(%e);
                None
            }
        })
        .collect();

    for subtitle in subtitles.iter() {
        info!("Subtitle language code: {}", subtitle.language_code());
    }

    add_subtitles_to_video_file(episode, &subtitles)
}

fn get_subtitles_pats(episode: &Episode, directory: &str) -> Result<Vec<PathBuf>> {
    // Find all files with "name.<language>.vtt" in the directory and that do not contain
    // "fixed.vtt"
    let pattern = format!("{}.*.vtt", episode.filename());
    let paths_results: Vec<Result<PathBuf, GlobError>> =
        glob::glob(&format!("{}/{}", directory, pattern))?.collect();

    if paths_results.is_empty() {
        warn!("No subtitle files found for episode {}", episode.filename());
        return Ok(vec![]);
    }

    let paths = paths_results
        .into_iter()
        .filter_map(|path_result| match path_result {
            Ok(path) => {
                if path.to_string_lossy().contains("fixed.vtt") {
                    debug!("Skipping already fixed subtitle file: {:?}", path.display());
                    return None;
                }

                info!("Found subtitle file: {:?}", path.display());
                Some(path)
            }
            Err(e) => {
                error!(error = %e, "Error finding subtitle file: {}", e);
                None
            }
        })
        .collect();

    Ok(paths)
}

fn clean_and_build_subtitle(path: PathBuf) -> Result<Subtitle> {
    // Open the file and remove all lines starting with "Region:" and save the
    // file as "name.fixed.vtt"
    let file = std::fs::File::open(&path)
        .map_err(|e| Error::IoError("Failed to open subtitle file".to_string(), e))?;
    let reader = BufReader::new(file);

    // Use the regex to filter out lines starting with "Region:"
    let cleaned_lines: Vec<String> = reader
        .lines()
        .filter_map(|line| {
            let line = line.unwrap();
            if REGEX_SUBTITLE_FIX.is_match(&line) {
                None
            } else {
                Some(line)
            }
        })
        .collect();
    let cleaned_content = cleaned_lines.join("\n");
    let new_path = path.with_extension("fixed.vtt");
    std::fs::write(&new_path, cleaned_content)
        .map_err(|e| Error::io_error("Failed to write cleaned subtitle file", e))?;

    let subtitle = Subtitle::new(new_path)?;
    debug!(
        ?subtitle,
        "Created fixed subtitle: {}",
        subtitle.path().display()
    );

    Ok(subtitle)
}

fn add_subtitles_to_video_file(episode: &Episode, subtitles: &Vec<Subtitle>) -> Result<()> {
    // Use ffmpeg to add subtitles to the video file
    let video_file = format!("{}.mp4", episode.filename());
    let output_file = format!("{}_fixed.mp4", episode.filename());

    let mut command = std::process::Command::new("ffmpeg");
    command.arg("-i").arg(video_file);
    for subtitle in subtitles {
        command.arg("-i").arg(subtitle.path());
    }
    command.arg("-c").arg("copy").arg(output_file);

    for (i, subtitle) in subtitles.iter().enumerate() {
        command.arg(format!(
            "-metadata:s:s:{} language={}",
            i + 1,
            subtitle.language_code()
        ));
    }

    let status = command.status()?;

    if !status.success() {
        return Err(
            Error::SubtitleError(format!("Running ffmpeg failed with status: {}", status)).into(),
        );
    }

    Ok(())
}
