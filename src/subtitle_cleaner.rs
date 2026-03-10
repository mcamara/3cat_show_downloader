//! VTT subtitle cleaning, fixing, and ASS conversion utilities.
//!
//! The VTT files from 3cat contain non-standard `Region:` headers and
//! `region:rN` attributes on cue timing lines. This module strips those
//! non-standard parts while preserving valid WEBVTT content including
//! inline styling tags such as `<c.white.background-black>`.
//!
//! Because neither MP4's `mov_text` codec nor ffmpeg's WebVTT encoder
//! preserve `<c.CLASS>` inline styling, this module also provides
//! [`convert_vtt_to_ass`] to translate cleaned VTT content into ASS
//! (Advanced SubStation Alpha) format.  ASS supports rich inline colour
//! overrides (`{\c&HBBGGRR&}`) and works well in Matroska containers.

use std::fmt::Write as FmtWrite;
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

// ---------------------------------------------------------------------------
// VTT → ASS conversion
// ---------------------------------------------------------------------------

/// ASS header template with a default style.
///
/// The default style uses white text with a semi-transparent black outline
/// and no background box, matching typical subtitle rendering.  Individual
/// cues override colours inline when the VTT source contains `<c.COLOR>`
/// class tags.
const ASS_HEADER: &str = "\
[Script Info]
Title: Converted from VTT
ScriptType: v4.00+
WrapStyle: 0
PlayResX: 1920
PlayResY: 1080

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,72,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,2,0,2,20,20,40,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
";

/// Maps a VTT CSS colour class name to an ASS BGR colour code.
///
/// Returns `None` for unrecognised class names.
fn css_color_to_ass_bgr(name: &str) -> Option<&'static str> {
    match name {
        "white" => Some("&H00FFFFFF&"),
        "yellow" => Some("&H0000FFFF&"),
        "cyan" => Some("&H00FFFF00&"),
        "green" => Some("&H0000FF00&"),
        "red" => Some("&H000000FF&"),
        "blue" => Some("&H00FF0000&"),
        "magenta" => Some("&H00FF00FF&"),
        _ => None,
    }
}

/// Maps a VTT CSS background class name to an ASS BGR colour code
/// suitable for the `\3c` (border colour) override.
fn css_background_to_ass_bgr(name: &str) -> Option<&'static str> {
    match name {
        "background-black" => Some("&H00000000&"),
        "background-white" => Some("&H00FFFFFF&"),
        "background-yellow" => Some("&H0000FFFF&"),
        "background-red" => Some("&H000000FF&"),
        _ => None,
    }
}

/// Converts a VTT timestamp (`HH:MM:SS.mmm`) to ASS format (`H:MM:SS.cc`).
///
/// ASS uses centiseconds (two decimal places) instead of milliseconds.
fn vtt_timestamp_to_ass(ts: &str) -> std::result::Result<String, String> {
    // Expected format: "HH:MM:SS.mmm"
    let parts: Vec<&str> = ts.split(':').collect();
    if parts.len() != 3 {
        return Err(format!("unexpected timestamp format: {ts}"));
    }

    let hours: u32 = parts[0]
        .parse()
        .map_err(|_| format!("invalid hours in timestamp: {ts}"))?;

    let minutes: u32 = parts[1]
        .parse()
        .map_err(|_| format!("invalid minutes in timestamp: {ts}"))?;

    let sec_parts: Vec<&str> = parts[2].split('.').collect();
    if sec_parts.len() != 2 {
        return Err(format!("missing milliseconds in timestamp: {ts}"));
    }

    let seconds: u32 = sec_parts[0]
        .parse()
        .map_err(|_| format!("invalid seconds in timestamp: {ts}"))?;

    let millis: u32 = sec_parts[1]
        .parse()
        .map_err(|_| format!("invalid milliseconds in timestamp: {ts}"))?;

    let centiseconds = millis / 10;

    Ok(format!(
        "{hours}:{minutes:02}:{seconds:02}.{centiseconds:02}"
    ))
}

/// Converts inline `<c.CLASS...>text</c>` VTT tags to ASS override tags.
///
/// For example, `<c.white.background-black>Hello</c>` becomes
/// `{\c&H00FFFFFF&\3c&H00000000&}Hello`.
///
/// Nested or overlapping tags are not supported (3cat VTT files don't use them).
fn convert_cue_text_to_ass(text: &str) -> String {
    let tag_re = Regex::new(r"<c\.([^>]+)>([^<]*)</c>").expect("valid regex");

    let result = tag_re.replace_all(text, |caps: &regex::Captures| {
        let classes = &caps[1];
        let content = &caps[2];

        let mut overrides = String::new();
        for class in classes.split('.') {
            if let Some(color) = css_color_to_ass_bgr(class) {
                let _ = write!(overrides, "\\c{color}");
            } else if let Some(bg) = css_background_to_ass_bgr(class) {
                let _ = write!(overrides, "\\3c{bg}");
            }
            // Unknown classes are silently ignored
        }

        if overrides.is_empty() {
            content.to_string()
        } else {
            format!("{{{overrides}}}{content}")
        }
    });

    // Replace VTT newlines with ASS newlines
    result.replace('\n', "\\N")
}

/// Converts cleaned VTT content to ASS (Advanced SubStation Alpha) format.
///
/// The input should already be cleaned by [`clean_vtt_content`] (region
/// headers and attributes removed).  This function parses the VTT cues,
/// converts timestamps to ASS format, and translates `<c.COLOR>` inline
/// styling to ASS override tags.
///
/// # Errors
///
/// Returns [`Error::SubtitleCleaning`] if the VTT content contains
/// malformed timing lines.
///
/// # Examples
///
/// ```
/// # use cat_show_downloader::subtitle_cleaner::{clean_vtt_content, convert_vtt_to_ass};
/// let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n<c.white.background-black>Hello</c>";
/// let ass = convert_vtt_to_ass(vtt).unwrap();
/// assert!(ass.contains("[Script Info]"));
/// assert!(ass.contains("{\\c&H00FFFFFF&\\3c&H00000000&}Hello"));
/// ```
pub fn convert_vtt_to_ass(vtt_content: &str) -> Result<String> {
    let mut ass = String::from(ASS_HEADER);

    let timing_re = Regex::new(r"^(\d{2}:\d{2}:\d{2}\.\d{3})\s*-->\s*(\d{2}:\d{2}:\d{2}\.\d{3})")
        .expect("valid regex");

    let lines: Vec<&str> = vtt_content.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i].trim();

        // Look for timing lines
        if let Some(caps) = timing_re.captures(line) {
            let start_vtt = &caps[1];
            let end_vtt = &caps[2];

            let start = vtt_timestamp_to_ass(start_vtt)
                .map_err(|e| Error::SubtitleCleaning(e.to_string()))?;
            let end = vtt_timestamp_to_ass(end_vtt)
                .map_err(|e| Error::SubtitleCleaning(e.to_string()))?;

            // Collect cue text lines until we hit a blank line or end of file
            i += 1;
            let mut cue_text = String::new();
            while i < lines.len() && !lines[i].is_empty() {
                if !cue_text.is_empty() {
                    cue_text.push('\n');
                }
                cue_text.push_str(lines[i]);
                i += 1;
            }

            let ass_text = convert_cue_text_to_ass(&cue_text);

            writeln!(ass, "Dialogue: 0,{start},{end},Default,,0,0,0,,{ass_text}",)
                .map_err(|e| Error::SubtitleCleaning(e.to_string()))?;
        } else {
            i += 1;
        }
    }

    Ok(ass)
}

/// Converts a cleaned VTT file to ASS format and writes it to the given path.
///
/// Reads the VTT content, converts it via [`convert_vtt_to_ass`], and writes
/// the result to `ass_path`.
///
/// # Errors
///
/// Returns an error if reading the VTT file, conversion, or writing fails.
pub fn convert_vtt_file_to_ass(vtt_path: &Path, ass_path: &Path) -> Result<()> {
    let vtt_content =
        std::fs::read_to_string(vtt_path).map_err(|e| Error::SubtitleCleaning(e.to_string()))?;

    let ass_content = convert_vtt_to_ass(&vtt_content)?;

    std::fs::write(ass_path, ass_content).map_err(|e| Error::SubtitleCleaning(e.to_string()))?;

    info!(
        "Converted {} to ASS: {}",
        vtt_path.display(),
        ass_path.display(),
    );
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

    // -------------------------------------------------------------------
    // VTT → ASS conversion tests
    // -------------------------------------------------------------------

    const CLEANED_VTT: &str = "\
WEBVTT

1
00:00:11.680 --> 00:00:14.920 line:88% align:center
<c.white.background-black>(home) \"Han passat dos anys
des de la guerra entre l'Armada,</c>

2
00:00:15.000 --> 00:00:17.560 line:88% align:center
<c.white.background-black>\"els Shichibukais
i els aliats d'en Barbablanca,</c>

3
00:00:17.920 --> 00:00:21.920 line:88% align:center
<c.white.background-black>per\u{f2} les seves conseq\u{fc}\u{e8}ncies encara
es noten... a tot el m\u{f3}n.\"</c>";

    #[test]
    fn test_should_convert_vtt_to_ass_with_header() {
        let ass = convert_vtt_to_ass(CLEANED_VTT).unwrap();
        assert!(ass.contains("[Script Info]"));
        assert!(ass.contains("[V4+ Styles]"));
        assert!(ass.contains("[Events]"));
        assert!(ass.contains("Format: Layer, Start, End,"));
    }

    #[test]
    fn test_should_convert_vtt_timestamps_to_ass_format() {
        let ass = convert_vtt_to_ass(CLEANED_VTT).unwrap();
        // VTT 00:00:11.680 → ASS 0:00:11.68
        assert!(ass.contains("0:00:11.68"));
        // VTT 00:00:14.920 → ASS 0:00:14.92
        assert!(ass.contains("0:00:14.92"));
        // VTT 00:00:15.000 → ASS 0:00:15.00
        assert!(ass.contains("0:00:15.00"));
    }

    #[test]
    fn test_should_convert_white_color_to_ass_bgr() {
        let ass = convert_vtt_to_ass(CLEANED_VTT).unwrap();
        assert!(ass.contains("\\c&H00FFFFFF&"));
    }

    #[test]
    fn test_should_convert_background_black_to_ass_border_color() {
        let ass = convert_vtt_to_ass(CLEANED_VTT).unwrap();
        assert!(ass.contains("\\3c&H00000000&"));
    }

    #[test]
    fn test_should_produce_dialogue_lines() {
        let ass = convert_vtt_to_ass(CLEANED_VTT).unwrap();
        let dialogue_count = ass.lines().filter(|l| l.starts_with("Dialogue:")).count();
        assert_eq!(dialogue_count, 3);
    }

    #[test]
    fn test_should_preserve_text_content_in_ass() {
        let ass = convert_vtt_to_ass(CLEANED_VTT).unwrap();
        assert!(ass.contains("Han passat dos anys"));
        assert!(ass.contains("els Shichibukais"));
    }

    #[test]
    fn test_should_convert_newlines_to_ass_newlines() {
        let ass = convert_vtt_to_ass(CLEANED_VTT).unwrap();
        // Multi-line cues should use \N (ASS newline)
        assert!(ass.contains("\\N"));
    }

    #[test]
    fn test_should_convert_yellow_color() {
        let vtt =
            "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n<c.yellow.background-black>Hello</c>";
        let ass = convert_vtt_to_ass(vtt).unwrap();
        assert!(ass.contains("\\c&H0000FFFF&"));
        assert!(ass.contains("\\3c&H00000000&"));
        assert!(ass.contains("Hello"));
    }

    #[test]
    fn test_should_convert_cyan_color() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n<c.cyan>Text</c>";
        let ass = convert_vtt_to_ass(vtt).unwrap();
        assert!(ass.contains("\\c&H00FFFF00&"));
        assert!(ass.contains("Text"));
    }

    #[test]
    fn test_should_convert_green_color() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n<c.green>Text</c>";
        let ass = convert_vtt_to_ass(vtt).unwrap();
        assert!(ass.contains("\\c&H0000FF00&"));
    }

    #[test]
    fn test_should_handle_plain_text_without_styling() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\nPlain text here";
        let ass = convert_vtt_to_ass(vtt).unwrap();
        assert!(ass.contains("Plain text here"));
        // Should not contain any colour overrides
        assert!(!ass.contains("\\c&H"));
    }

    #[test]
    fn test_should_handle_mixed_styled_and_plain_text() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\nBefore <c.white>styled</c> after";
        let ass = convert_vtt_to_ass(vtt).unwrap();
        assert!(ass.contains("Before {\\c&H00FFFFFF&}styled after"));
    }

    #[test]
    fn test_should_convert_vtt_file_to_ass_file() {
        let dir = std::env::temp_dir().join("vtt_to_ass_test");
        let _ = std::fs::create_dir_all(&dir);

        let vtt_path = dir.join("test.vtt");
        let ass_path = dir.join("test.ass");
        std::fs::write(&vtt_path, CLEANED_VTT).unwrap();

        convert_vtt_file_to_ass(&vtt_path, &ass_path).unwrap();

        assert!(ass_path.exists());
        let content = std::fs::read_to_string(&ass_path).unwrap();
        assert!(content.contains("[Script Info]"));
        assert!(content.contains("Dialogue:"));
        assert!(content.contains("\\c&H00FFFFFF&"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_should_handle_timestamp_conversion_edge_cases() {
        // Test hours > 0
        let vtt = "WEBVTT\n\n1\n01:23:45.678 --> 02:00:00.000\nText";
        let ass = convert_vtt_to_ass(vtt).unwrap();
        assert!(ass.contains("1:23:45.67"));
        assert!(ass.contains("2:00:00.00"));
    }

    #[test]
    fn test_should_ignore_unknown_css_classes() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.000\n<c.unknown-class>Text</c>";
        let ass = convert_vtt_to_ass(vtt).unwrap();
        // Unknown class should be ignored, text preserved without overrides
        assert!(ass.contains("Text"));
    }

    #[test]
    fn test_should_produce_complete_ass_dialogue_format() {
        let vtt = "WEBVTT\n\n1\n00:00:01.000 --> 00:00:02.500\nHello world";
        let ass = convert_vtt_to_ass(vtt).unwrap();
        assert!(ass.contains("Dialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,Hello world"));
    }
}
