use tracing::error;

pub mod downloader;
pub mod http_client;
pub mod info_3cat;
pub mod models;
pub mod subtitles_fix;
pub mod utils;

pub fn check_requirements() -> bool {
    // Check whether yt-dlp and ffmpeg are installed
    let yt_dlp_installed = which::which("yt-dlp").is_ok();
    let ffmpeg_installed = which::which("ffmpeg").is_ok();

    if !yt_dlp_installed {
        error!("yt-dlp is not installed. Please install it to use this program.");
    }

    if !ffmpeg_installed {
        error!("ffmpeg is not installed. Please install it to use this program.");
    }

    yt_dlp_installed && ffmpeg_installed
}
