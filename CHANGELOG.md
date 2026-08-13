## Unreleased

- Add conservative Plex metadata mode for MIC3: TheTVDB Aired Order is used only for unique exact or `>= 0.95` fuzzy title matches; every other item retains its 3Cat chapter number.
- Generate Plex-compatible `Season XX` folders, filenames, `tvshow.nfo`, per-episode NFO files, and redundant Matroska tags sourced from 3Cat metadata.
- Add dry-run/apply repair workflow with collision checks, original-tag exports, recovery manifest, sidecar migration, leftover quarantine, and per-file MKV/ffprobe validation.
- Add a native GUI launcher (`--gui`, or open the executable without arguments) with folder selection, download options, Plex repair controls, and live process output.
- Mark the primary Matroska audio track as Catalan (`cat` / `ca`, track name `Català`) and validate it with ffprobe after Plex tagging.

## 1.3.0 - 2026-08-05

- Every downloaded file now ends up as a `.mkv` (Matroska) container regardless of which container yt-dlp / the built-in HTTP downloader produced, whether subtitles are embedded, and whether re-encoding runs. A new `ffmpeg::remux_to_mkv` lossless stream-copy remux is the final stage of every download path (built-in HTTP and yt-dlp). The remux is idempotent — already-`.mkv` files short-circuit — and failures are tolerated (warn + keep original). The legacy `--reencode off --skip-subtitles` path now also produces `.mkv`.
- `--reencode <preset>` now prefers GPU encoders over software. A new `ffmpeg::probe_encoders()` helper probes `ffmpeg -encoders` once at startup (cached in a `OnceCell`) and records the best available backend per codec family. Each preset now picks the preferred backend: NVIDIA NVENC for `light`/`balanced` (H.264 NVENC CQ 26/23), NVENC for `max` (HEVC NVENC CQ 28), with a documented CPU fallback to `libx264`/`libx265` (CRF 23/26/28 with `veryfast`/`ultrafast` preset). The probe also surfaces AMD AMF and Intel QSV when present, so the pipeline is ready for those backends without API changes. Re-encode progress messages show the active backend, e.g. `hevc_nvenc CQ 28 (GPU)` vs `libx265 CRF 28 (CPU)`.
- New public types in `ffmpeg.rs` (`VcodecBackend`, `VcodecTarget`, `EncoderCapabilities`, `VcodecChoice`) plus a `transcode::pick_vcodec` resolver that the re-encode path consults; the resolver is exhaustively unit-tested for every preset × backend combination.
- Subtitle downloads are now tolerant: when the 3cat API is missing a subtitle URL, the subtitle stream fails to download, or `subtitle_cleaner` fails to read/clean the VTT for a given episode, the failure is logged as a `tracing::warn!` and the video is still saved. The previously failing episode can be re-downloaded with `--skip-subtitles` to silence the warning. Pass `--strict-subtitles` to restore the legacy fail-fast behaviour (any subtitle error aborts the whole batch).
- Add `--auto-naming` (and optional `--season N`, default 1): when enabled, files are saved as `Mic - <episode_slug> - S<season:02>E<ep:02> (<ext>)` and organised under `Temporada XX/` (episodes) or `Pel·lícules/` (movies) subdirectories inside the user-supplied output directory. The auto-naming subdirectory is created on demand.
- Add `--reencode <preset>` (default `off`): post-download re-encoding with ffmpeg to reduce file size. Presets: `light` (H.264 CRF 26, ~-35% size), `balanced` (H.264 CRF 23, ~-50% size, recommended), `max` (H.265 CRF 28, ~-65% size, slower). Audio + subtitle streams are copied untouched. Requires ffmpeg on PATH. Failures are tolerated — original file is preserved when re-encoding fails.
- Fix `decoding error: request body read error: error decoding response body` when fetching TV show episode lists and single-episode metadata from the 3cat API. The endpoint returns heterogeneous items where some entries omit `programa` (and a few other consumed fields); making every consumed field `Option<T>` with `#[serde(default)]` lets the decoder tolerate the missing fields instead of failing the whole 339-item response on the first absence. Items still missing the minimum data needed to build a `MediaItem` (id + permatitle) are now skipped. The single-episode endpoint is parsed as `serde_json::Value` and its `media.url` is walked defensively because it can be either a single object or an array of objects.
- Preserve the decoding error chain end-to-end: the previous `Error::Decoding(String)` discarded the underlying reqwest/serde_json source. The variant now carries a `context` string (URL + operation) and a boxed `#[source]` error so the chain survives into logs and the CLI error message.
- Extract `HttpClient::format_response` body decoding into a pure `decode_body(status, bytes)` helper and add 7 regression tests covering: valid payload, optional-field absence (mirrors `missing field 'programa'`), empty body, HTML body, schema drift, non-2xx with parseable error, and non-2xx with unparseable body.

## 1.2.0 - 2026-03-19

- Add yt-dlp as an optional download backend: when `yt-dlp` is found on PATH it is used instead of the built-in HTTP downloader. The CCMA/3cat extractor built into yt-dlp handles format selection and subtitle extraction internally, so no prior API call is made. Subtitle embedding (`SubtitleMode::Embed`) forces Matroska output (`--merge-output-format mkv`) for consistency with the ffmpeg path. If yt-dlp is not installed the existing behaviour is unchanged.

## 1.1.0 - 2026-03-18

- Move all module declarations, orchestration logic, and progress writer infrastructure from `main.rs` to `lib.rs`, reducing the binary entry point to a thin ~20-line wrapper that only handles tracing setup and runtime construction.

## 1.0.1 - 2026-03-13

- Handle episodes without subtitles gracefully: the `subtitles` field in the API response is now optional. When subtitles are unavailable and the user has not passed `--skip-subtitles`, a clear error is returned naming the episode and suggesting the flag.

## 1.0.0 - 2026-03-13

- Add movie download support: the tool now automatically detects whether a slug is a TV show or a movie and downloads accordingly.
- Change the slug from a named parameter (`--tv-show-slug` / `-t`) to a positional argument for simpler invocation (e.g. `./cat_show_downloader bola-de-drac -d ./output/`).
- Rename internal `Episode` model to `MediaItem` to support both TV show episodes and movies through a unified download pipeline.
- Restructure `tv_show` and `movie` modules into their own directories with dedicated `api_structs` submodules.

## 0.1.0

- Add parallel downloading support
- Embed subtitles to downloaded files and create MKV files using ffmpeg
- Add progress bar support

## 0.0.2 - 2024-12-02

- Make id retrieval more robust

## 0.0.1 - 2024-12-02

- First release
