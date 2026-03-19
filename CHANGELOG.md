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
