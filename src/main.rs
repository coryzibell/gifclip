mod config;
mod setup;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

#[derive(Debug, Clone, ValueEnum, PartialEq)]
enum OutputFormat {
    Gif,
    Webm,
    Mp4,
}

#[derive(Parser)]
#[command(name = "gifclip")]
#[command(about = "Download a YouTube clip and export as GIF/video with burned-in subtitles")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run interactive setup to configure tool sources
    #[arg(long)]
    setup: bool,

    /// YouTube URL
    #[arg(required_unless_present_any = ["command", "setup"])]
    url: Option<String>,

    /// Start timestamp (e.g., "1:30" or "00:01:30" or "90")
    #[arg(required_unless_present_any = ["command", "setup"])]
    start: Option<String>,

    /// End timestamp (e.g., "1:35" or "00:01:35" or "95")
    #[arg(required_unless_present_any = ["command", "setup"])]
    end: Option<String>,

    /// Output filename (auto-generated from video title if not specified)
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output format
    #[arg(short, long, value_enum, default_value = "gif")]
    format: OutputFormat,

    /// Width in pixels (height scales proportionally)
    #[arg(short, long, default_value = "480")]
    width: u32,

    /// Frames per second
    #[arg(long, default_value = "15")]
    fps: u32,

    /// Subtitle language code
    #[arg(long, default_value = "en")]
    lang: String,

    /// Skip subtitles
    #[arg(long)]
    no_subs: bool,

    /// Quality for lossy formats (1-100, higher is better). For gif, reduces colors.
    #[arg(short, long, default_value = "80")]
    quality: u32,
}

#[derive(Subcommand)]
enum Commands {
    /// Configure gifclip (tool sources, etc.)
    Setup,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // Handle setup flag or subcommand
    if cli.setup || matches!(cli.command, Some(Commands::Setup)) {
        setup::run_setup()?;
        return Ok(());
    }

    // Ensure we have required args for clip mode
    let url = cli.url.as_ref().context("URL is required")?;
    let start = cli.start.as_ref().context("Start timestamp is required")?;
    let end = cli.end.as_ref().context("End timestamp is required")?;

    // Ensure tools are configured
    let config = setup::ensure_setup()?;

    let start_secs = parse_timestamp(start)?;
    let end_secs = parse_timestamp(end)?;

    if end_secs <= start_secs {
        bail!("End time must be after start time");
    }

    let duration = end_secs - start_secs;
    println!(
        "Clipping {:.1}s from {} to {}",
        duration, start, end
    );

    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let temp_path = temp_dir.path();

    let yt_dlp = config.yt_dlp_path()?;
    let ffmpeg = config.ffmpeg_path()?;

    // Get video title for auto-naming
    let video_title = get_video_title(&yt_dlp, url)?;
    println!("Video: {}", video_title);

    // Download video
    println!("Downloading video...");
    let video_path = temp_path.join("video.mp4");
    let mut dl_cmd = Command::new(&yt_dlp);
    dl_cmd
        .arg("-f")
        .arg("b[ext=mp4]/b")
        .arg("-o")
        .arg(&video_path)
        .arg("--no-playlist");

    if !cli.no_subs {
        dl_cmd
            .arg("--write-sub")
            .arg("--write-auto-sub")
            .arg("--sub-lang")
            .arg(&cli.lang)
            .arg("--convert-subs")
            .arg("srt");
    }

    dl_cmd.arg(url);

    let dl_status = dl_cmd.status().context("Failed to run yt-dlp")?;
    if !dl_status.success() {
        bail!("yt-dlp failed to download video");
    }

    // Find subtitle file
    let sub_path = find_subtitle_file(temp_path, &cli.lang);
    let has_subs = !cli.no_subs && sub_path.is_some();

    if !cli.no_subs && !has_subs {
        eprintln!("Warning: No subtitles found, proceeding without them");
    }

    // Determine output path
    let output_path = match &cli.output {
        Some(p) => p.clone(),
        None => {
            let safe_title = sanitize_filename(&video_title);
            let ext = match cli.format {
                OutputFormat::Gif => "gif",
                OutputFormat::Webm => "webm",
                OutputFormat::Mp4 => "mp4",
            };
            PathBuf::from(format!(
                "{}_{}-{}.{}",
                safe_title,
                format_timestamp(start_secs),
                format_timestamp(end_secs),
                ext
            ))
        }
    };

    // Build and run ffmpeg
    println!("Generating {}...", output_path.display());

    match cli.format {
        OutputFormat::Gif => encode_gif(&ffmpeg, &video_path, &output_path, &sub_path, &cli, start_secs, duration)?,
        OutputFormat::Webm => encode_webm(&ffmpeg, &video_path, &output_path, &sub_path, &cli, start_secs, duration)?,
        OutputFormat::Mp4 => encode_mp4(&ffmpeg, &video_path, &output_path, &sub_path, &cli, start_secs, duration)?,
    }

    println!("Created: {}", output_path.display());

    Ok(())
}

fn get_video_title(yt_dlp: &Path, url: &str) -> Result<String> {
    let output = Command::new(yt_dlp)
        .arg("--get-title")
        .arg("--no-playlist")
        .arg(url)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .context("Failed to get video title")?;

    if !output.status.success() {
        bail!("Failed to fetch video title");
    }

    let title = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(title)
}

fn sanitize_filename(name: &str) -> String {
    let re = Regex::new(r#"[<>:"/\\|?*]"#).unwrap();
    let sanitized = re.replace_all(name, "_");
    sanitized.chars().take(50).collect()
}

fn format_timestamp(secs: f64) -> String {
    let mins = (secs / 60.0).floor() as u32;
    let secs = (secs % 60.0).floor() as u32;
    format!("{}m{}s", mins, secs)
}

fn build_subtitle_filter(sub_path: &Option<PathBuf>) -> Option<String> {
    sub_path.as_ref().map(|subs| {
        let sub_escaped = subs
            .to_string_lossy()
            .replace('\\', "\\\\")
            .replace(':', "\\:")
            .replace("'", "\\'");
        format!("subtitles='{}'", sub_escaped)
    })
}

fn encode_gif(
    ffmpeg: &Path,
    video_path: &Path,
    output_path: &Path,
    sub_path: &Option<PathBuf>,
    cli: &Cli,
    start_secs: f64,
    duration: f64,
) -> Result<()> {
    let mut filters = vec![
        format!("fps={}", cli.fps),
        format!("scale={}:-1:flags=lanczos", cli.width),
    ];

    if let Some(sub_filter) = build_subtitle_filter(sub_path) {
        filters.insert(0, sub_filter);
    }

    let max_colors = 16 + ((cli.quality as f32 / 100.0) * 240.0) as u32;

    let filter_base = filters.join(",");
    let filter_complex = format!(
        "{},split[s0][s1];[s0]palettegen=max_colors={}[p];[s1][p]paletteuse=dither=bayer",
        filter_base, max_colors
    );

    let status = Command::new(ffmpeg)
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-ss")
        .arg(format!("{}", start_secs))
        .arg("-t")
        .arg(format!("{}", duration))
        .arg("-vf")
        .arg(&filter_complex)
        .arg(output_path)
        .status()
        .context("Failed to run ffmpeg")?;

    if !status.success() {
        bail!("ffmpeg failed to create GIF");
    }

    Ok(())
}

fn encode_webm(
    ffmpeg: &Path,
    video_path: &Path,
    output_path: &Path,
    sub_path: &Option<PathBuf>,
    cli: &Cli,
    start_secs: f64,
    duration: f64,
) -> Result<()> {
    let mut filters = vec![
        format!("fps={}", cli.fps),
        format!("scale={}:-1", cli.width),
    ];

    if let Some(sub_filter) = build_subtitle_filter(sub_path) {
        filters.insert(0, sub_filter);
    }

    let filter_str = filters.join(",");
    let crf = 63 - ((cli.quality as f32 / 100.0) * 53.0) as u32;

    let status = Command::new(ffmpeg)
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-ss")
        .arg(format!("{}", start_secs))
        .arg("-t")
        .arg(format!("{}", duration))
        .arg("-vf")
        .arg(&filter_str)
        .arg("-c:v")
        .arg("libvpx-vp9")
        .arg("-crf")
        .arg(format!("{}", crf))
        .arg("-b:v")
        .arg("0")
        .arg("-an")
        .arg(output_path)
        .status()
        .context("Failed to run ffmpeg")?;

    if !status.success() {
        bail!("ffmpeg failed to create WebM");
    }

    Ok(())
}

fn encode_mp4(
    ffmpeg: &Path,
    video_path: &Path,
    output_path: &Path,
    sub_path: &Option<PathBuf>,
    cli: &Cli,
    start_secs: f64,
    duration: f64,
) -> Result<()> {
    let mut filters = vec![
        format!("fps={}", cli.fps),
        format!("scale={}:-1", cli.width),
    ];

    if let Some(sub_filter) = build_subtitle_filter(sub_path) {
        filters.insert(0, sub_filter);
    }

    let filter_str = filters.join(",");
    let crf = 51 - ((cli.quality as f32 / 100.0) * 41.0) as u32;

    let status = Command::new(ffmpeg)
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-ss")
        .arg(format!("{}", start_secs))
        .arg("-t")
        .arg(format!("{}", duration))
        .arg("-vf")
        .arg(&filter_str)
        .arg("-c:v")
        .arg("libx264")
        .arg("-crf")
        .arg(format!("{}", crf))
        .arg("-preset")
        .arg("medium")
        .arg("-an")
        .arg("-movflags")
        .arg("+faststart")
        .arg(output_path)
        .status()
        .context("Failed to run ffmpeg")?;

    if !status.success() {
        bail!("ffmpeg failed to create MP4");
    }

    Ok(())
}

fn parse_timestamp(ts: &str) -> Result<f64> {
    if let Ok(secs) = ts.parse::<f64>() {
        return Ok(secs);
    }

    let re = Regex::new(r"^(?:(\d+):)?(\d+):(\d+(?:\.\d+)?)$").unwrap();
    if let Some(caps) = re.captures(ts) {
        let hours: f64 = caps.get(1).map_or(0.0, |m| m.as_str().parse().unwrap_or(0.0));
        let minutes: f64 = caps[2].parse().unwrap_or(0.0);
        let seconds: f64 = caps[3].parse().unwrap_or(0.0);
        return Ok(hours * 3600.0 + minutes * 60.0 + seconds);
    }

    bail!("Invalid timestamp format: {}. Use MM:SS, HH:MM:SS, or seconds", ts)
}

fn find_subtitle_file(dir: &Path, lang: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;

    let mut srt_files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "srt")
                && p.to_string_lossy().contains(lang)
        })
        .collect();

    srt_files.sort_by_key(|p| p.to_string_lossy().len());
    srt_files.into_iter().next()
}
