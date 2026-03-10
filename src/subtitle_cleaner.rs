//! VTT subtitle cleaning and fixing utilities.
//!
//! The VTT files from 3cat contain non-standard `Region:` headers and
//! `region:rN` attributes on cue timing lines. This module strips those
//! non-standard parts while preserving valid WEBVTT content including
//! inline styling tags such as `<c.white.background-black>`.

use std::path::Path;

use regex::Regex;
use tracing::{info, warn};

use crate::error::{Error, Result};

/// Cleans raw VTT content by removing non-standard Region headers and
/// region attributes from cue timing lines.
///
/// Specifically:
/// - Removes all `Region: …` lines from the header block
/// - Strips `region:rN` (and variants like `region:r12`) from `-->` timing lines
/// - Removes blank lines left behind by stripped Region headers
/// - Preserves all other content including `<c.*>` inline styling tags
///
/// # Examples
///
/// ```
/// # use cat_show_downloader::subtitle_cleaner::clean_vtt_content;
/// let raw = "WEBVTT\n\nRegion: id=r1 width=100%\n\n1\n00:00:01.000 --> 00:00:02.000 region:r1\nHello";
/// let cleaned = clean_vtt_content(raw);
/// assert!(cleaned.contains("WEBVTT"));
/// assert!(!cleaned.contains("Region:"));
/// assert!(!cleaned.contains("region:r1"));
/// ```
pub fn clean_vtt_content(content: &str) -> String {
    let region_line_re = Regex::new(r"(?m)^Region:.*$\n?").expect("valid regex");
    let without_regions = region_line_re.replace_all(content, "");

    let region_attr_re = Regex::new(r"\s*region:r\d+").expect("valid regex");
    let without_region_attrs = region_attr_re.replace_all(&without_regions, "");

    // Collapse runs of 3+ newlines (left by stripped Region blocks) into double newlines
    let excess_newlines_re = Regex::new(r"\n{3,}").expect("valid regex");
    let cleaned = excess_newlines_re.replace_all(&without_region_attrs, "\n\n");

    cleaned.into_owned()
}

/// Reads a VTT file, cleans its content, and writes it back in-place.
///
/// # Errors
///
/// Returns an error if reading or writing the file fails.
pub fn clean_vtt_file(path: &Path) -> Result<()> {
    let content =
        std::fs::read_to_string(path).map_err(|e| Error::SubtitleCleaning(e.to_string()))?;

    let cleaned = clean_vtt_content(&content);

    std::fs::write(path, cleaned).map_err(|e| Error::SubtitleCleaning(e.to_string()))?;

    info!("Cleaned subtitle file: {}", path.display());
    Ok(())
}

/// Finds all `.vtt` files in the given directory and cleans each one in-place.
///
/// Logs a warning for any file that fails to clean and continues with the rest.
///
/// # Errors
///
/// Returns an error if reading the directory itself fails.
pub fn fix_existing_subtitles(directory: &str) -> Result<()> {
    let dir_path = Path::new(directory);
    let entries =
        std::fs::read_dir(dir_path).map_err(|e| Error::SubtitleCleaning(e.to_string()))?;

    let mut cleaned_count = 0u32;
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Failed to read directory entry: {e}");
                continue;
            }
        };

        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("vtt") {
            match clean_vtt_file(&path) {
                Ok(()) => cleaned_count += 1,
                Err(e) => warn!("Failed to clean {}: {e}", path.display()),
            }
        }
    }

    info!("Cleaned {cleaned_count} existing subtitle file(s)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_VTT: &str = "\
WEBVTT

Region: id=r1 width=100% lines=3 regionanchor=0%,96% viewportanchor=0%,100% scroll=none 
Region: id=r2 width=100% lines=3 regionanchor=0%,96% viewportanchor=0%,100% scroll=none 
Region: id=r3 width=98% lines=3 regionanchor=3%,88% viewportanchor=0%,100% scroll=none 
Region: id=r4 width=100% lines=1 regionanchor=0%,96% viewportanchor=0%,100% scroll=none 
Region: id=r5 width=100% lines=3 regionanchor=0%,88% viewportanchor=0%,100% scroll=none 
Region: id=r6 width=100% lines=3 regionanchor=0%,79% viewportanchor=0%,100% scroll=none 
Region: id=r7 width=98% lines=3 regionanchor=3%,96% viewportanchor=0%,100% scroll=none 
Region: id=r8 width=98% lines=1 regionanchor=3%,96% viewportanchor=0%,100% scroll=none 
Region: id=r9 width=98% lines=3 regionanchor=3%,88% viewportanchor=0%,100% scroll=none 


1
00:00:11.680 --> 00:00:14.920 region:r1 line:88% align:center
<c.white.background-black>(home) \"Han passat dos anys
des de la guerra entre l'Armada,</c>

2
00:00:15.000 --> 00:00:17.560 region:r2 line:88% align:center
<c.white.background-black>\"els Shichibukais
i els aliats d'en Barbablanca,</c>

3
00:00:17.920 --> 00:00:21.920 region:r1 line:88% align:center
<c.white.background-black>per\u{f2} les seves conseq\u{fc}\u{e8}ncies encara
es noten... a tot el m\u{f3}n.\"</c>";

    #[test]
    fn test_should_remove_region_header_lines() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(!cleaned.contains("Region:"));
    }

    #[test]
    fn test_should_keep_webvtt_header() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(cleaned.starts_with("WEBVTT"));
    }

    #[test]
    fn test_should_strip_region_attributes_from_timing_lines() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(!cleaned.contains("region:r1"));
        assert!(!cleaned.contains("region:r2"));
    }

    #[test]
    fn test_should_preserve_other_timing_attributes() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(cleaned.contains("line:88%"));
        assert!(cleaned.contains("align:center"));
    }

    #[test]
    fn test_should_preserve_inline_styling_tags() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(cleaned.contains("<c.white.background-black>"));
        assert!(cleaned.contains("</c>"));
    }

    #[test]
    fn test_should_preserve_cue_text_content() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(cleaned.contains("Han passat dos anys"));
        assert!(cleaned.contains("els Shichibukais"));
        assert!(cleaned.contains("es noten... a tot el m"));
    }

    #[test]
    fn test_should_preserve_cue_numbers() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(cleaned.contains("\n1\n"));
        assert!(cleaned.contains("\n2\n"));
        assert!(cleaned.contains("\n3\n"));
    }

    #[test]
    fn test_should_not_leave_excessive_blank_lines() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(!cleaned.contains("\n\n\n"));
    }

    #[test]
    fn test_should_produce_correct_timing_lines() {
        let cleaned = clean_vtt_content(SAMPLE_VTT);
        assert!(cleaned.contains("00:00:11.680 --> 00:00:14.920 line:88% align:center"));
        assert!(cleaned.contains("00:00:15.000 --> 00:00:17.560 line:88% align:center"));
    }

    #[test]
    fn test_should_handle_vtt_with_no_regions() {
        let simple_vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\nHello world";
        let cleaned = clean_vtt_content(simple_vtt);
        assert_eq!(cleaned, simple_vtt);
    }

    #[test]
    fn test_should_clean_vtt_file_in_place() {
        let dir = std::env::temp_dir().join("vtt_test_clean");
        let _ = std::fs::create_dir_all(&dir);
        let file_path = dir.join("test.vtt");
        std::fs::write(&file_path, SAMPLE_VTT).unwrap();

        clean_vtt_file(&file_path).unwrap();

        let cleaned = std::fs::read_to_string(&file_path).unwrap();
        assert!(!cleaned.contains("Region:"));
        assert!(!cleaned.contains("region:r1"));
        assert!(cleaned.contains("<c.white.background-black>"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_should_fix_existing_subtitles_in_directory() {
        let dir = std::env::temp_dir().join("vtt_test_fix_existing");
        let _ = std::fs::create_dir_all(&dir);

        let vtt1 = dir.join("episode1.vtt");
        let vtt2 = dir.join("episode2.vtt");
        let mp4 = dir.join("episode1.mp4");
        std::fs::write(&vtt1, SAMPLE_VTT).unwrap();
        std::fs::write(&vtt2, SAMPLE_VTT).unwrap();
        std::fs::write(&mp4, "not a vtt").unwrap();

        fix_existing_subtitles(dir.to_str().unwrap()).unwrap();

        let cleaned1 = std::fs::read_to_string(&vtt1).unwrap();
        let cleaned2 = std::fs::read_to_string(&vtt2).unwrap();
        let mp4_content = std::fs::read_to_string(&mp4).unwrap();

        assert!(!cleaned1.contains("Region:"));
        assert!(!cleaned2.contains("Region:"));
        assert_eq!(mp4_content, "not a vtt"); // Non-vtt files untouched

        let _ = std::fs::remove_dir_all(&dir);
    }
}
