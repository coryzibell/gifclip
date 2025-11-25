use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};
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
struct Args {
    /// YouTube URL
    url: String,

    /// Start timestamp (e.g., "1:30" or "00:01:30" or "90")
    start: String,

    /// End timestamp (e.g., "1:35" or "00:01:35" or "95")
    end: String,

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

fn main() -> Result<()> {
    let args = Args::parse();

    check_dependencies()?;

    let start_secs = parse_timestamp(&args.start)?;
    let end_secs = parse_timestamp(&args.end)?;

    if end_secs <= start_secs {
        bail!("End time must be after start time");
    }

    let duration = end_secs - start_secs;
    println!(
        "Clipping {:.1}s from {} to {}",
        duration, args.start, args.end
    );

    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let temp_path = temp_dir.path();

    // Get video title for auto-naming
    let video_title = get_video_title(&args.url)?;
    println!("Video: {}", video_title);

    // Download video
    println!("Downloading video...");
    let video_path = temp_path.join("video.mp4");
    let mut dl_cmd = Command::new("yt-dlp");
    dl_cmd
        .arg("-f")
        .arg("b[ext=mp4]/b") // best mp4, fallback to best available
        .arg("-o")
        .arg(&video_path)
        .arg("--no-playlist");

    if !args.no_subs {
        dl_cmd
            .arg("--write-sub") // manual subs (preferred)
            .arg("--write-auto-sub") // auto-generated fallback
            .arg("--sub-lang")
            .arg(&args.lang)
            .arg("--convert-subs")
            .arg("srt");
    }

    dl_cmd.arg(&args.url);

    let dl_status = dl_cmd.status().context("Failed to run yt-dlp")?;
    if !dl_status.success() {
        bail!("yt-dlp failed to download video");
    }

    // Find subtitle file - yt-dlp may name it differently depending on source
    let sub_path = find_subtitle_file(temp_path, &args.lang);
    let has_subs = !args.no_subs && sub_path.is_some();

    if !args.no_subs && !has_subs {
        eprintln!("Warning: No subtitles found, proceeding without them");
    }

    // Determine output path
    let output_path = match &args.output {
        Some(p) => p.clone(),
        None => {
            let safe_title = sanitize_filename(&video_title);
            let ext = match args.format {
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

    match args.format {
        OutputFormat::Gif => encode_gif(&video_path, &output_path, &sub_path, &args, start_secs, duration)?,
        OutputFormat::Webm => encode_webm(&video_path, &output_path, &sub_path, &args, start_secs, duration)?,
        OutputFormat::Mp4 => encode_mp4(&video_path, &output_path, &sub_path, &args, start_secs, duration)?,
    }

    println!("Created: {}", output_path.display());

    Ok(())
}

fn get_video_title(url: &str) -> Result<String> {
    let output = Command::new("yt-dlp")
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
    // Truncate to reasonable length
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
    video_path: &Path,
    output_path: &Path,
    sub_path: &Option<PathBuf>,
    args: &Args,
    start_secs: f64,
    duration: f64,
) -> Result<()> {
    let mut filters = vec![
        format!("fps={}", args.fps),
        format!("scale={}:-1:flags=lanczos", args.width),
    ];

    if let Some(sub_filter) = build_subtitle_filter(sub_path) {
        filters.insert(0, sub_filter);
    }

    // Adjust palette based on quality (max_colors: 16-256)
    let max_colors = 16 + ((args.quality as f32 / 100.0) * 240.0) as u32;

    let filter_base = filters.join(",");
    let filter_complex = format!(
        "{},split[s0][s1];[s0]palettegen=max_colors={}[p];[s1][p]paletteuse=dither=bayer",
        filter_base, max_colors
    );

    let status = Command::new("ffmpeg")
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
    video_path: &Path,
    output_path: &Path,
    sub_path: &Option<PathBuf>,
    args: &Args,
    start_secs: f64,
    duration: f64,
) -> Result<()> {
    let mut filters = vec![
        format!("fps={}", args.fps),
        format!("scale={}:-1", args.width),
    ];

    if let Some(sub_filter) = build_subtitle_filter(sub_path) {
        filters.insert(0, sub_filter);
    }

    let filter_str = filters.join(",");

    // VP9 CRF: 0-63, lower is better. Map quality 1-100 to CRF 63-10
    let crf = 63 - ((args.quality as f32 / 100.0) * 53.0) as u32;

    let status = Command::new("ffmpeg")
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
        .arg("-an") // no audio for clips
        .arg(output_path)
        .status()
        .context("Failed to run ffmpeg")?;

    if !status.success() {
        bail!("ffmpeg failed to create WebM");
    }

    Ok(())
}

fn encode_mp4(
    video_path: &Path,
    output_path: &Path,
    sub_path: &Option<PathBuf>,
    args: &Args,
    start_secs: f64,
    duration: f64,
) -> Result<()> {
    let mut filters = vec![
        format!("fps={}", args.fps),
        format!("scale={}:-1", args.width),
    ];

    if let Some(sub_filter) = build_subtitle_filter(sub_path) {
        filters.insert(0, sub_filter);
    }

    let filter_str = filters.join(",");

    // H.264 CRF: 0-51, lower is better. Map quality 1-100 to CRF 51-10
    let crf = 51 - ((args.quality as f32 / 100.0) * 41.0) as u32;

    let status = Command::new("ffmpeg")
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
        .arg("-an") // no audio for clips
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

fn check_dependencies() -> Result<()> {
    if which::which("yt-dlp").is_err() {
        bail!("yt-dlp not found. Install with: nix-shell -p yt-dlp");
    }
    if which::which("ffmpeg").is_err() {
        bail!("ffmpeg not found. Install with: nix-shell -p ffmpeg");
    }
    Ok(())
}

fn parse_timestamp(ts: &str) -> Result<f64> {
    // Handle pure seconds
    if let Ok(secs) = ts.parse::<f64>() {
        return Ok(secs);
    }

    // Handle MM:SS or HH:MM:SS
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
    // yt-dlp names subs differently based on source:
    // - Manual: video.en.srt
    // - Auto-generated: video.en.srt (after conversion, same name)
    // But sometimes includes source info like video.en-orig.srt
    let entries = std::fs::read_dir(dir).ok()?;

    let mut srt_files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension().is_some_and(|ext| ext == "srt")
                && p.to_string_lossy().contains(lang)
        })
        .collect();

    // Prefer non-auto-generated (shorter name usually)
    srt_files.sort_by_key(|p| p.to_string_lossy().len());
    srt_files.into_iter().next()
}
