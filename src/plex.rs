//! Plex-compatible naming, NFO generation, Matroska tags, and collection repair.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::Serialize;
use tracing::{info, warn};
use unidecode::unidecode;

use crate::error::{Error, Result};
use crate::http_client::HttpClientTrait;
use crate::models::MediaItem;
use crate::tv_show::episodes;

const FUZZY_THRESHOLD: f64 = 0.95;
const SERIES_TITLE: &str = "MIC3";

#[derive(Clone, Debug)]
struct TvdbEpisode {
    season: i32,
    episode: i32,
    id: u64,
    title: String,
}

#[derive(Debug, Serialize)]
struct Manifest {
    version: u8,
    created_unix_seconds: u64,
    tvdb_series_id: u32,
    operations: Vec<ManifestOperation>,
}

#[derive(Debug, Serialize)]
struct ManifestOperation {
    source: String,
    destination: String,
    threecat_id: i32,
    season: i32,
    episode: i32,
    tvdb_episode_id: Option<u64>,
}

#[derive(Clone, Debug)]
struct RepairOperation {
    source: PathBuf,
    destination: PathBuf,
    item: MediaItem,
}

/// Converts a 3Cat date into the ISO date consumed by Plex NFO files.
pub(crate) fn parse_3cat_date(value: &str) -> Option<String> {
    let date = value.split_whitespace().next()?;
    let mut parts = date.split('/');
    let day = parts.next()?;
    let month = parts.next()?;
    let year = parts.next()?;
    if parts.next().is_some() || day.len() != 2 || month.len() != 2 || year.len() != 4 {
        return None;
    }
    Some(format!("{year}-{month}-{day}"))
}

/// Returns the dedicated Plex series directory below a user output root.
pub(crate) fn series_root(root: &Path, tvdb_series_id: u32) -> PathBuf {
    root.join("Plex TV")
        .join(format!("{SERIES_TITLE} {{tvdb-{tvdb_series_id}}}"))
}

/// Checks external tools required by apply/tagging workflows.
pub(crate) fn preflight(require_mkv_tools: bool) -> anyhow::Result<()> {
    if !require_mkv_tools {
        return Ok(());
    }
    for tool in ["mkvpropedit", "mkvextract", "mkvinfo", "ffprobe"] {
        let executable = tool_path(tool).ok_or_else(|| {
            anyhow::anyhow!("{tool} was not found on PATH; install MKVToolNix before apply")
        })?;
        let version_arg = if tool == "ffprobe" {
            "-version"
        } else {
            "--version"
        };
        let status = Command::new(executable).arg(version_arg).status();
        if !status.is_ok_and(|s| s.success()) {
            anyhow::bail!("{tool} was not found on PATH; install MKVToolNix before apply");
        }
    }
    Ok(())
}

fn tool_path(tool: &str) -> Option<PathBuf> {
    if Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        return Some(PathBuf::from(tool));
    }
    #[cfg(windows)]
    {
        for directory in [r"C:\Program Files\MKVToolNix", r"C:\ffmpeg\bin"] {
            let candidate = PathBuf::from(directory).join(format!("{tool}.exe"));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Applies unique, high-confidence TheTVDB Aired Order matches to 3Cat items.
pub(crate) async fn enrich_with_tvdb(items: &mut [MediaItem], tvdb_series_id: u32) -> Result<()> {
    let catalog = fetch_tvdb_catalog(tvdb_series_id).await?;
    if catalog.is_empty() {
        warn!("TheTVDB returned no parseable episodes; using 3Cat numbering for every item");
        return Ok(());
    }
    apply_matches(items, &catalog);
    Ok(())
}

async fn fetch_tvdb_catalog(tvdb_series_id: u32) -> Result<Vec<TvdbEpisode>> {
    // MIC3's stable TheTVDB slug contains its numeric ID. A parser failure is
    // deliberately non-destructive: callers retain 3Cat numbering.
    let url = format!("https://thetvdb.com/series/{tvdb_series_id}-show/allseasons/official");
    let html = reqwest::get(&url)
        .await
        .map_err(|e| Error::Plex(format!("failed to fetch TheTVDB Aired Order: {e}")))?
        .text()
        .await
        .map_err(|e| Error::Plex(format!("failed to read TheTVDB Aired Order: {e}")))?;
    parse_tvdb_catalog(&html)
}

fn parse_tvdb_catalog(html: &str) -> Result<Vec<TvdbEpisode>> {
    let re = Regex::new(
        r#"(?is)S(?P<s>\d{2})E(?P<e>\d{2,4}).{0,800}?/episodes/(?P<id>\d+)[^>]*>\s*(?P<title>[^<]+?)\s*</a>"#,
    )?;
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for caps in re.captures_iter(html) {
        let season = caps["s"].parse()?;
        let episode = caps["e"].parse()?;
        let id = caps["id"]
            .parse()
            .map_err(|e| Error::Plex(format!("invalid TheTVDB episode ID: {e}")))?;
        if seen.insert((season, episode, id)) {
            result.push(TvdbEpisode {
                season,
                episode,
                id,
                title: decode_html(&caps["title"]),
            });
        }
    }
    Ok(result)
}

fn decode_html(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&#039;", "'")
        .replace("&apos;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .trim()
        .to_string()
}

fn apply_matches(items: &mut [MediaItem], catalog: &[TvdbEpisode]) {
    let mut proposed = Vec::with_capacity(items.len());
    for item in items.iter() {
        proposed.push(best_match(&item.title, catalog));
    }
    let mut counts: HashMap<(i32, i32), usize> = HashMap::new();
    for candidate in proposed.iter().flatten() {
        *counts
            .entry((candidate.season, candidate.episode))
            .or_default() += 1;
    }
    for (item, candidate) in items.iter_mut().zip(proposed) {
        let Some(candidate) = candidate else { continue };
        if counts[&(candidate.season, candidate.episode)] != 1 {
            warn!(
                "Ambiguous TheTVDB destination for '{}'; keeping 3Cat numbering",
                item.title
            );
            continue;
        }
        if let Some(plex) = &mut item.plex {
            plex.season = candidate.season;
            plex.episode = candidate.episode;
            plex.tvdb_episode_id = Some(candidate.id);
            info!(
                "TheTVDB match: '{}' -> S{:02}E{:02}",
                item.title, candidate.season, candidate.episode
            );
        }
    }
}

fn best_match<'a>(title: &str, catalog: &'a [TvdbEpisode]) -> Option<&'a TvdbEpisode> {
    let target = normalize(title);
    let exact: Vec<_> = catalog
        .iter()
        .filter(|candidate| normalize(&candidate.title) == target)
        .collect();
    if exact.len() == 1 {
        return exact.first().copied();
    }
    if !exact.is_empty() {
        return None;
    }
    let mut ranked: Vec<_> = catalog
        .iter()
        .map(|candidate| (similarity(&target, &normalize(&candidate.title)), candidate))
        .filter(|(score, _)| *score >= FUZZY_THRESHOLD)
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (best_score, best) = ranked.first().copied()?;
    if ranked
        .get(1)
        .is_some_and(|(score, _)| (best_score - score).abs() < f64::EPSILON)
    {
        return None;
    }
    Some(best)
}

fn normalize(value: &str) -> String {
    let ascii = unidecode(value).to_lowercase();
    ascii
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn similarity(left: &str, right: &str) -> f64 {
    let max_len = left.chars().count().max(right.chars().count());
    if max_len == 0 {
        return 1.0;
    }
    1.0 - levenshtein(left, right) as f64 / max_len as f64
}

fn levenshtein(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (i, l) in left.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, r) in right.iter().enumerate() {
            current.push(
                (current[j] + 1)
                    .min(previous[j + 1] + 1)
                    .min(previous[j] + usize::from(l != *r)),
            );
        }
        previous = current;
    }
    previous[right.len()]
}

/// Writes NFO and Matroska tags for files downloaded in Plex mode.
pub(crate) async fn finalize_downloads(
    items: &[MediaItem],
    directory: &Path,
    tvdb_series_id: u32,
) -> Result<()> {
    write_tvshow_nfo(directory, tvdb_series_id)?;
    for item in items {
        let path = media_path(directory, item)?;
        if !path.exists() {
            return Err(Error::Plex(format!(
                "downloaded MKV not found: {}",
                path.display()
            )));
        }
        write_episode_nfo(&path, item)?;
        tag_and_validate(&path, item)?;
    }
    Ok(())
}

/// Audits or repairs an existing MIC3 collection.
pub(crate) async fn repair_existing<T: HttpClientTrait>(
    root: &Path,
    tv_show_id: i32,
    tvdb_series_id: u32,
    apply: bool,
    http_client: &Arc<T>,
) -> anyhow::Result<()> {
    let mut items = episodes::get_episodes(http_client, tv_show_id).await?;
    enrich_with_tvdb(&mut items, tvdb_series_id).await?;
    let files = collect_mkvs(root)?;
    let operations = build_repair_operations(root, tvdb_series_id, &files, &items)?;
    validate_destinations(&operations)?;
    let matched_tvdb = operations
        .iter()
        .filter(|op| {
            op.item
                .plex
                .as_ref()
                .and_then(|p| p.tvdb_episode_id)
                .is_some()
        })
        .count();
    info!(
        "Repair audit: {} MKV, {} TheTVDB matches, {} 3Cat fallbacks",
        operations.len(),
        matched_tvdb,
        operations.len() - matched_tvdb
    );
    println!(
        "Repair audit: {} MKV, {} TheTVDB matches, {} 3Cat fallbacks",
        operations.len(),
        matched_tvdb,
        operations.len() - matched_tvdb
    );
    for op in operations.iter().take(12) {
        info!("{} -> {}", op.source.display(), op.destination.display());
    }
    if !apply {
        info!("PLAN only: no files were changed");
        println!("PLAN only: no files were changed");
        return Ok(());
    }
    apply_repair(root, tvdb_series_id, &operations)?;
    Ok(())
}

fn collect_mkvs(root: &Path) -> Result<Vec<PathBuf>> {
    fn recurse(dir: &Path, result: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                if path.file_name() != Some(OsStr::new("Plex TV")) {
                    recurse(&path, result)?;
                }
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mkv"))
            {
                result.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    recurse(root, &mut files)
        .map_err(|e| Error::Plex(format!("failed to scan {}: {e}", root.display())))?;
    files.sort();
    Ok(files)
}

fn build_repair_operations(
    root: &Path,
    tvdb_series_id: u32,
    files: &[PathBuf],
    items: &[MediaItem],
) -> Result<Vec<RepairOperation>> {
    let old_name =
        Regex::new(r"(?i)^Mic - (?P<slug>.+?) - S(?P<season>\d{1,3})E(?P<episode>\d{1,4})")?;
    let mut by_slug: HashMap<String, Vec<&MediaItem>> = HashMap::new();
    for item in items {
        by_slug
            .entry(normalize(&item.title))
            .or_default()
            .push(item);
    }
    let target_root = series_root(root, tvdb_series_id);
    let mut result = Vec::new();
    for source in files {
        let name = source
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| Error::Plex(format!("non-UTF-8 filename: {}", source.display())))?;
        let caps = old_name
            .captures(name)
            .ok_or_else(|| Error::Plex(format!("unrecognized MIC3 filename: {name}")))?;
        let key = normalize(&caps["slug"].replace('-', " "));
        let source_episode: i32 = caps["episode"].parse()?;
        let exact = by_slug.get(&key).filter(|values| values.len() == 1);
        let candidate = exact
            .and_then(|values| values.first().copied())
            .or_else(|| {
                items.iter().find(|item| {
                    item.episode_number == Some(source_episode)
                        && similarity(&key, &normalize(&item.title)) >= 0.70
                })
            })
            .or_else(|| unique_source_fuzzy_match(&key, items));
        let item = candidate
            .ok_or_else(|| Error::Plex(format!("no safe 3Cat metadata match for {name}")))?
            .clone();
        let destination = media_path(&target_root, &item)?;
        result.push(RepairOperation {
            source: source.clone(),
            destination,
            item,
        });
    }
    Ok(result)
}

fn unique_source_fuzzy_match<'a>(key: &str, items: &'a [MediaItem]) -> Option<&'a MediaItem> {
    let mut ranked: Vec<_> = items
        .iter()
        .map(|item| (similarity(key, &normalize(&item.title)), item))
        .filter(|(score, _)| *score >= 0.85)
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (best_score, best) = ranked.first().copied()?;
    if ranked
        .get(1)
        .is_some_and(|(score, _)| (best_score - score).abs() < f64::EPSILON)
    {
        return None;
    }
    Some(best)
}

fn validate_destinations(operations: &[RepairOperation]) -> Result<()> {
    let mut destinations = HashSet::new();
    let mut episode_keys = HashSet::new();
    for op in operations {
        let plex = op
            .item
            .plex
            .as_ref()
            .ok_or_else(|| Error::Plex("missing Plex identity".into()))?;
        if !destinations.insert(op.destination.clone()) {
            return Err(Error::Plex(format!(
                "duplicate destination: {}",
                op.destination.display()
            )));
        }
        if !episode_keys.insert((plex.season, plex.episode)) {
            return Err(Error::Plex(format!(
                "duplicate episode key S{:02}E{:02}",
                plex.season, plex.episode
            )));
        }
        if op.destination.exists() && op.destination != op.source {
            return Err(Error::Plex(format!(
                "destination already exists: {}",
                op.destination.display()
            )));
        }
    }
    Ok(())
}

fn apply_repair(root: &Path, tvdb_series_id: u32, operations: &[RepairOperation]) -> Result<()> {
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Plex(e.to_string()))?
        .as_secs();
    let audit_dir = root.join(format!("plex-repair-{created}"));
    let tag_dir = audit_dir.join("original-tags");
    std::fs::create_dir_all(&tag_dir).map_err(|e| Error::Plex(e.to_string()))?;
    let manifest = Manifest {
        version: 1,
        created_unix_seconds: created,
        tvdb_series_id,
        operations: operations
            .iter()
            .map(|op| {
                let plex = op.item.plex.as_ref().expect("validated Plex metadata");
                ManifestOperation {
                    source: op.source.to_string_lossy().into_owned(),
                    destination: op.destination.to_string_lossy().into_owned(),
                    threecat_id: op.item.id,
                    season: plex.season,
                    episode: plex.episode,
                    tvdb_episode_id: plex.tvdb_episode_id,
                }
            })
            .collect(),
    };
    let manifest_json =
        serde_json::to_string_pretty(&manifest).map_err(|e| Error::Plex(e.to_string()))?;
    std::fs::write(audit_dir.join("manifest.json"), manifest_json)
        .map_err(|e| Error::Plex(e.to_string()))?;
    let series_dir = series_root(root, tvdb_series_id);
    std::fs::create_dir_all(&series_dir).map_err(|e| Error::Plex(e.to_string()))?;
    write_tvshow_nfo(&series_dir, tvdb_series_id)?;
    for (index, op) in operations.iter().enumerate() {
        export_tags(
            &op.source,
            &tag_dir.join(format!("{:04}-{}.xml", index + 1, op.item.id)),
        )?;
    }
    info!(
        "Exported original tags for all {} MKV files",
        operations.len()
    );
    for (index, op) in operations.iter().enumerate() {
        if let Some(parent) = op.destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Plex(e.to_string()))?;
        }
        std::fs::rename(&op.source, &op.destination)
            .map_err(|e| Error::Plex(format!("failed to move {}: {e}", op.source.display())))?;
        write_episode_nfo(&op.destination, &op.item)?;
        tag_and_validate(&op.destination, &op.item)?;
        info!(
            "Repaired {}/{}: {}",
            index + 1,
            operations.len(),
            op.destination.display()
        );
    }
    let old_dir = root.join("Temporada 01");
    if old_dir.exists() {
        move_sidecars_and_leftovers(&old_dir, operations, &audit_dir)?;
        std::fs::remove_dir(&old_dir)
            .map_err(|e| Error::Plex(format!("old folder is not empty: {e}")))?;
    }
    let manifest_path = audit_dir.join("manifest.json");
    info!(
        "Repair complete. Recovery manifest: {}",
        manifest_path.display()
    );
    Ok(())
}

fn move_sidecars_and_leftovers(
    old_dir: &Path,
    operations: &[RepairOperation],
    audit_dir: &Path,
) -> Result<()> {
    let name_pattern =
        Regex::new(r"(?i)^Mic - (?P<slug>.+?) - S(?P<season>\d{1,3})E(?P<episode>\d{1,4})")?;
    let files: Vec<_> = std::fs::read_dir(old_dir)
        .map_err(|e| Error::Plex(e.to_string()))?
        .filter_map(|entry| entry.ok().map(|value| value.path()))
        .filter(|path| path.is_file())
        .collect();
    let leftovers = audit_dir.join("leftovers");
    for source in files {
        let name = source
            .file_name()
            .and_then(OsStr::to_str)
            .unwrap_or_default();
        let mut destination = None;
        if name.to_ascii_lowercase().ends_with(".ca.vtt")
            && let Some(caps) = name_pattern.captures(name)
        {
            let slug = &caps["slug"];
            let episode = caps["episode"].parse::<i32>().ok();
            let candidates: Vec<_> = operations
                .iter()
                .filter(|operation| {
                    operation
                        .source
                        .file_name()
                        .and_then(OsStr::to_str)
                        .and_then(|value| name_pattern.captures(value))
                        .is_some_and(|value| &value["slug"] == slug)
                })
                .collect();
            let operation = if candidates.len() == 1 {
                candidates.first().copied()
            } else {
                candidates.into_iter().find(|operation| {
                    operation
                        .source
                        .file_name()
                        .and_then(OsStr::to_str)
                        .and_then(|value| name_pattern.captures(value))
                        .and_then(|value| value["episode"].parse::<i32>().ok())
                        == episode
                })
            };
            if let Some(operation) = operation {
                let stem = operation
                    .destination
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .ok_or_else(|| Error::Plex("invalid destination filename".into()))?;
                destination = Some(
                    operation
                        .destination
                        .with_file_name(format!("{stem}.ca.vtt")),
                );
            }
        }
        let destination =
            destination.unwrap_or_else(|| leftovers.join(source.file_name().unwrap_or_default()));
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::Plex(e.to_string()))?;
        }
        if destination.exists() {
            return Err(Error::Plex(format!(
                "sidecar destination exists: {}",
                destination.display()
            )));
        }
        std::fs::rename(&source, &destination).map_err(|e| {
            Error::Plex(format!(
                "failed to preserve sidecar {} -> {}: {e}",
                source.display(),
                destination.display()
            ))
        })?;
    }
    Ok(())
}

fn media_path(directory: &Path, item: &MediaItem) -> Result<PathBuf> {
    let subdir = item
        .subdirectory(true)
        .ok_or_else(|| Error::Plex("episode has no Plex season".into()))?;
    Ok(directory.join(subdir).join(item.filename("mkv", true)?))
}

fn write_tvshow_nfo(directory: &Path, tvdb_series_id: u32) -> Result<()> {
    std::fs::create_dir_all(directory).map_err(|e| Error::Plex(e.to_string()))?;
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>\n<tvshow>\n  <title>{SERIES_TITLE}</title>\n  <plot>Programa infantil de 3Cat protagonitzat pel Mic i els seus amics.</plot>\n  <status>Continuing</status>\n  <studio>3Cat</studio>\n  <genre>Children</genre>\n  <uniqueid type=\"tvdb\" default=\"true\">{tvdb_series_id}</uniqueid>\n</tvshow>\n"
    );
    std::fs::write(directory.join("tvshow.nfo"), xml).map_err(|e| Error::Plex(e.to_string()))
}

fn write_episode_nfo(video: &Path, item: &MediaItem) -> Result<()> {
    let plex = item
        .plex
        .as_ref()
        .ok_or_else(|| Error::Plex("missing Plex metadata".into()))?;
    let id = plex.tvdb_episode_id.map_or_else(
        || {
            format!(
                "<uniqueid type=\"3cat\" default=\"true\">{}</uniqueid>",
                item.id
            )
        },
        |id| format!("<uniqueid type=\"tvdb\" default=\"true\">{id}</uniqueid>"),
    );
    let aired = plex.aired.as_ref().map_or(String::new(), |v| {
        format!("\n  <aired>{}</aired>", xml_escape(v))
    });
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\" ?>\n<episodedetails>\n  <title>{}</title>\n  <season>{}</season>\n  <episode>{}</episode>\n  <plot>{}</plot>{aired}\n  {id}\n</episodedetails>\n",
        xml_escape(&item.title),
        plex.season,
        plex.episode,
        xml_escape(plex.plot.as_deref().unwrap_or("")),
    );
    std::fs::write(video.with_extension("nfo"), xml).map_err(|e| Error::Plex(e.to_string()))
}

fn tag_and_validate(video: &Path, item: &MediaItem) -> Result<()> {
    let plex = item
        .plex
        .as_ref()
        .ok_or_else(|| Error::Plex("missing Plex metadata".into()))?;
    let tags_path = video.with_extension("plex-tags.xml");
    let tags = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Tags><Tag><Targets/><Simple><Name>TITLE</Name><String>{}</String></Simple><Simple><Name>DESCRIPTION</Name><String>{}</String></Simple><Simple><Name>DATE_RELEASED</Name><String>{}</String></Simple><Simple><Name>SEASON_NUMBER</Name><String>{}</String></Simple><Simple><Name>EPISODE_NUMBER</Name><String>{}</String></Simple><Simple><Name>3CAT_ID</Name><String>{}</String></Simple><Simple><Name>TVDB_ID</Name><String>{}</String></Simple><Simple><Name>DURATION</Name><String>{}</String></Simple></Tag></Tags>\n",
        xml_escape(&item.title),
        xml_escape(plex.plot.as_deref().unwrap_or("")),
        xml_escape(plex.aired.as_deref().unwrap_or("")),
        plex.season,
        plex.episode,
        item.id,
        plex.tvdb_episode_id
            .map_or_else(String::new, |v| v.to_string()),
        xml_escape(plex.duration.as_deref().unwrap_or("")),
    );
    std::fs::write(&tags_path, tags).map_err(|e| Error::Plex(e.to_string()))?;
    let mkvpropedit =
        tool_path("mkvpropedit").ok_or_else(|| Error::Plex("mkvpropedit not found".into()))?;
    let status = Command::new(mkvpropedit)
        .arg(video)
        .args(catalan_audio_edit_args())
        .args(["--tags", &format!("all:{}", tags_path.display())])
        .status();
    let _ = std::fs::remove_file(&tags_path);
    if !status.is_ok_and(|s| s.success()) {
        return Err(Error::Plex(format!(
            "mkvpropedit failed for {}",
            video.display()
        )));
    }
    for (tool, args) in [
        ("mkvinfo", vec![video.as_os_str()]),
        (
            "ffprobe",
            vec![
                OsStr::new("-v"),
                OsStr::new("error"),
                OsStr::new("-show_format"),
                video.as_os_str(),
            ],
        ),
    ] {
        let executable = tool_path(tool).ok_or_else(|| Error::Plex(format!("{tool} not found")))?;
        if !Command::new(executable)
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            return Err(Error::Plex(format!(
                "{tool} validation failed for {}",
                video.display()
            )));
        }
    }
    validate_catalan_audio(video)?;
    Ok(())
}

fn catalan_audio_edit_args() -> [&'static str; 8] {
    [
        "--edit",
        "track:a1",
        "--set",
        "language=cat",
        "--set",
        "language-ietf=ca",
        "--set",
        "name=Català",
    ]
}

fn validate_catalan_audio(video: &Path) -> Result<()> {
    let ffprobe = tool_path("ffprobe").ok_or_else(|| Error::Plex("ffprobe not found".into()))?;
    let output = Command::new(ffprobe)
        .args([
            OsStr::new("-v"),
            OsStr::new("error"),
            OsStr::new("-select_streams"),
            OsStr::new("a:0"),
            OsStr::new("-show_entries"),
            OsStr::new("stream_tags=language,title"),
            OsStr::new("-of"),
            OsStr::new("default=noprint_wrappers=1"),
        ])
        .arg(video)
        .output()
        .map_err(|e| Error::Plex(e.to_string()))?;
    let tags = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() || !tags.lines().any(|line| line == "TAG:language=cat") {
        return Err(Error::Plex(format!(
            "Catalan audio language validation failed for {}",
            video.display()
        )));
    }
    Ok(())
}

fn export_tags(video: &Path, destination: &Path) -> Result<()> {
    let mkvextract =
        tool_path("mkvextract").ok_or_else(|| Error::Plex("mkvextract not found".into()))?;
    let output = Command::new(mkvextract)
        .arg(video)
        .arg("tags")
        .output()
        .map_err(|e| Error::Plex(e.to_string()))?;
    if !output.status.success() {
        return Err(Error::Plex(format!(
            "mkvextract failed for {}",
            video.display()
        )));
    }
    std::fs::write(destination, output.stdout).map_err(|e| Error::Plex(e.to_string()))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tvdb(season: i32, episode: i32, id: u64, title: &str) -> TvdbEpisode {
        TvdbEpisode {
            season,
            episode,
            id,
            title: title.into(),
        }
    }

    #[test]
    fn parses_3cat_date() {
        assert_eq!(
            parse_3cat_date("12/09/2023 17:14:47").as_deref(),
            Some("2023-09-12")
        );
        assert_eq!(parse_3cat_date("bad"), None);
    }

    #[test]
    fn exact_and_high_fuzzy_matches_only() {
        let catalog = vec![
            tvdb(1, 4, 44, "El capità Paperina"),
            tvdb(3, 42, 342, "Cap, panxa, genolls i peus"),
        ];
        assert_eq!(
            best_match("El capità Paperina", &catalog).map(|v| v.id),
            Some(44)
        );
        assert_eq!(
            best_match("El capita Paperina!", &catalog).map(|v| v.id),
            Some(44)
        );
        assert_eq!(
            best_match("Cantem cap panxa genolls i peus", &catalog).map(|v| v.id),
            None
        );
    }

    #[test]
    fn ambiguous_exact_title_is_rejected() {
        let catalog = vec![tvdb(1, 1, 1, "Repetit"), tvdb(2, 1, 2, "Repetit")];
        assert!(best_match("Repetit", &catalog).is_none());
    }

    #[test]
    fn xml_is_utf8_safe_and_escaped() {
        assert_eq!(
            xml_escape("Mic & Mosca <amics>"),
            "Mic &amp; Mosca &lt;amics&gt;"
        );
    }

    #[test]
    fn marks_primary_audio_track_as_catalan() {
        assert_eq!(
            catalan_audio_edit_args(),
            [
                "--edit",
                "track:a1",
                "--set",
                "language=cat",
                "--set",
                "language-ietf=ca",
                "--set",
                "name=Català",
            ]
        );
    }

    #[test]
    fn parses_tvdb_fixture() {
        let html = r#"<h4>S01E04 <a href="/series/280190-show/episodes/12345">El capità Paperina</a></h4>"#;
        let rows = parse_tvdb_catalog(html).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].episode, 4);
        assert_eq!(rows[0].id, 12345);
    }
}
