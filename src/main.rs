mod config;
mod setup;
mod srt;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use regex::Regex;
use std::fs;
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
#[command(version)]
#[command(about = "Create GIFs/videos with burned-in subtitles from YouTube, local files, or URLs")]
#[command(long_about = "Create GIFs/videos with burned-in subtitles from YouTube, local files, or URLs.

TIMESTAMP MODE:
  gifclip <INPUT> <START> <END>

  Clip a video using specific timestamps.

  Examples:
    gifclip \"https://youtube.com/watch?v=...\" 1:30 1:45
    gifclip movie.mp4 0:45 0:59 -f mp4 -w 720
    gifclip \"https://example.com/video.mp4\" 0:10 0:20

DIALOGUE MODE:
  gifclip <INPUT> --from \"dialogue text\" [--to \"ending text\"]

  Search subtitles for dialogue and clip around it automatically.

  Single quote (2s default padding):
    gifclip \"URL\" --from \"I'll be back\"

  Dialogue range (0.5s default padding):
    gifclip \"URL\" --from \"Here's looking\" --to \"kid\"

  Custom padding:
    gifclip \"URL\" --from \"quote\" --pad 3
    gifclip \"URL\" --from \"quote\" --pad-before 1 --pad-after 5

INPUT TYPES:
  - YouTube URL: Downloads via yt-dlp, auto-fetches subtitles
  - Local file: Uses embedded subs or looks for matching .srt file
  - Direct URL: Downloads video, uses --subs if provided

SETUP:
  gifclip --setup

  Configure whether to use system-installed tools (yt-dlp, ffmpeg)
  or download managed copies to ~/.gifclip/tools/")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run interactive setup to configure tool sources
    #[arg(long)]
    setup: bool,

    /// Input: YouTube URL, local file path, or direct video URL
    #[arg(required_unless_present_any = ["command", "setup"])]
    input: Option<String>,

    /// Start timestamp (e.g., "1:30" or "00:01:30" or "90")
    #[arg(required_unless_present_any = ["command", "setup", "from"])]
    start: Option<String>,

    /// End timestamp (e.g., "1:35" or "00:01:35" or "95")
    #[arg(required_unless_present_any = ["command", "setup", "from"])]
    end: Option<String>,

    /// External subtitle file path or URL (overrides auto-detected subs)
    #[arg(long)]
    subs: Option<String>,

    /// Starting dialogue text to search for in subtitles (alternative to timestamps)
    #[arg(long, conflicts_with_all = ["start", "end"])]
    from: Option<String>,

    /// Ending dialogue text (optional - if omitted, clips around --from with padding)
    #[arg(long, requires = "from")]
    to: Option<String>,

    /// Padding in seconds around dialogue clips (default: 0.5s with --to, 2s without)
    #[arg(long, conflicts_with_all = ["pad_before", "pad_after"])]
    pad: Option<f64>,

    /// Padding before the dialogue starts (overrides --pad)
    #[arg(long)]
    pad_before: Option<f64>,

    /// Padding after the dialogue ends (overrides --pad)
    #[arg(long)]
    pad_after: Option<f64>,

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

    // Ensure tools are configured
    let config = setup::ensure_setup()?;

    let temp_dir = TempDir::new().context("Failed to create temp directory")?;
    let temp_path = temp_dir.path();

    let ffmpeg = config.ffmpeg_path()?;

    let input = cli.input.as_ref().context("Input is required")?;

    // Determine input type and get video + subtitles
    let (video_path, video_title, sub_path) = if is_url(input) && is_youtube_url(input) {
        // YouTube mode - use yt-dlp
        let yt_dlp = config.yt_dlp_path()?;

        let video_title = get_video_title(&yt_dlp, input)?;
        println!("Video: {}", video_title);

        // Download video (always get subs for dialogue mode, or if user wants them)
        let need_subs = cli.subs.is_none() && (cli.from.is_some() || !cli.no_subs);

        println!("Downloading video...");
        let video_path = temp_path.join("video.mp4");
        let mut dl_cmd = Command::new(&yt_dlp);
        dl_cmd
            .arg("-f")
            .arg("b[ext=mp4]/b")
            .arg("-o")
            .arg(&video_path)
            .arg("--no-playlist");

        if need_subs {
            dl_cmd
                .arg("--write-sub")
                .arg("--write-auto-sub")
                .arg("--sub-lang")
                .arg(&cli.lang)
                .arg("--convert-subs")
                .arg("srt");
        }

        dl_cmd.arg(input);

        let dl_status = dl_cmd.status().context("Failed to run yt-dlp")?;
        if !dl_status.success() {
            bail!("yt-dlp failed to download video");
        }

        // Handle subtitles
        let sub_path = if let Some(ref subs_input) = cli.subs {
            Some(resolve_subs_input(subs_input, temp_path)?)
        } else {
            find_subtitle_file(temp_path, &cli.lang)
        };

        (video_path, video_title, sub_path)
    } else if is_url(input) {
        // Direct URL mode - download video, check embedded subs only
        println!("Downloading video...");
        let ext = Path::new(input)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("mp4");
        let video_path = temp_path.join(format!("video.{}", ext));
        download_file(input, &video_path)?;

        let video_title = get_filename_from_url(input);
        println!("Video: {}", video_title);

        // Handle subtitles - explicit subs or try embedded
        let sub_path = if let Some(ref subs_input) = cli.subs {
            Some(resolve_subs_input(subs_input, temp_path)?)
        } else if !cli.no_subs {
            let extracted_subs = temp_path.join("extracted.srt");
            if extract_embedded_subs(&ffmpeg, &video_path, &extracted_subs)? {
                println!("Extracted embedded subtitles");
                Some(extracted_subs)
            } else {
                None
            }
        } else {
            None
        };

        (video_path, video_title, sub_path)
    } else {
        // Local file mode - check embedded subs, then adjacent .srt
        let video_path = PathBuf::from(input);
        if !video_path.exists() {
            bail!("Input file does not exist: {}", input);
        }

        let video_title = get_filename_from_path(input);
        println!("Video: {}", video_title);

        // Handle subtitles - explicit, embedded, or adjacent file
        let sub_path = if let Some(ref subs_input) = cli.subs {
            Some(resolve_subs_input(subs_input, temp_path)?)
        } else if !cli.no_subs {
            // First try embedded subs
            let extracted_subs = temp_path.join("extracted.srt");
            if extract_embedded_subs(&ffmpeg, &video_path, &extracted_subs)? {
                println!("Extracted embedded subtitles");
                Some(extracted_subs)
            } else {
                // Look for adjacent subtitle file with same name
                find_adjacent_subtitle(&video_path)
            }
        } else {
            None
        };

        (video_path, video_title, sub_path)
    };

    // Determine start/end times
    let (start_secs, end_secs) = if let Some(ref from_text) = cli.from {
        // Dialogue mode - search subtitles
        let sub_file = sub_path.as_ref()
            .context("Subtitles required for dialogue search but none found")?;

        let entries = srt::parse_srt(sub_file)?;

        let from_entry = srt::find_dialogue(&entries, from_text)
            .with_context(|| format!("Could not find starting dialogue: \"{}\"", from_text))?;

        let (start, end, default_pad) = if let Some(ref to_text) = cli.to {
            // Range mode: from dialogue to dialogue
            let to_entry = srt::find_dialogue(&entries, to_text)
                .with_context(|| format!("Could not find ending dialogue: \"{}\"", to_text))?;

            if to_entry.end < from_entry.start {
                bail!("Ending dialogue appears before starting dialogue");
            }

            (from_entry.start, to_entry.end, 0.5)
        } else {
            // Single quote mode: just the one subtitle entry
            (from_entry.start, from_entry.end, 2.0)
        };

        let pad_before = cli.pad_before.or(cli.pad).unwrap_or(default_pad);
        let pad_after = cli.pad_after.or(cli.pad).unwrap_or(default_pad);
        let start_padded = (start - pad_before).max(0.0);
        let end_padded = end + pad_after;

        println!(
            "Found dialogue at {:.1}s - {:.1}s (padding: {:.1}s before, {:.1}s after)",
            start, end, pad_before, pad_after
        );

        (start_padded, end_padded)
    } else {
        // Timestamp mode
        let start = cli.start.as_ref().context("Start timestamp is required")?;
        let end = cli.end.as_ref().context("End timestamp is required")?;

        let start_secs = parse_timestamp(start)?;
        let end_secs = parse_timestamp(end)?;

        if end_secs <= start_secs {
            bail!("End time must be after start time");
        }

        (start_secs, end_secs)
    };

    let duration = end_secs - start_secs;
    println!(
        "Clipping {:.1}s from {:.1}s to {:.1}s",
        duration, start_secs, end_secs
    );

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

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn is_youtube_url(s: &str) -> bool {
    s.contains("youtube.com") || s.contains("youtu.be")
}

fn download_file(url: &str, dest: &Path) -> Result<()> {
    let response = reqwest::blocking::get(url)
        .with_context(|| format!("Failed to download {}", url))?;

    if !response.status().is_success() {
        bail!("Failed to download {}: HTTP {}", url, response.status());
    }

    let bytes = response.bytes()
        .with_context(|| format!("Failed to read response from {}", url))?;

    fs::write(dest, &bytes)
        .with_context(|| format!("Failed to write to {}", dest.display()))?;

    Ok(())
}

fn extract_embedded_subs(ffmpeg: &Path, video_path: &Path, output_path: &Path) -> Result<bool> {
    // Try to extract embedded subtitles using ffmpeg
    let status = Command::new(ffmpeg)
        .arg("-y")
        .arg("-i")
        .arg(video_path)
        .arg("-map")
        .arg("0:s:0")  // First subtitle stream
        .arg(output_path)
        .stderr(Stdio::null())
        .status()
        .context("Failed to run ffmpeg for subtitle extraction")?;

    Ok(status.success())
}

fn get_filename_from_path(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("video")
        .to_string()
}

fn get_filename_from_url(url: &str) -> String {
    // Try to extract filename from URL path
    url.split('/')
        .last()
        .and_then(|s| s.split('?').next())
        .map(|s| {
            Path::new(s)
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or(s)
                .to_string()
        })
        .unwrap_or_else(|| "video".to_string())
}

fn resolve_subs_input(subs_input: &str, temp_path: &Path) -> Result<PathBuf> {
    if is_url(subs_input) {
        println!("Downloading subtitles...");
        let ext = Path::new(subs_input)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("srt");
        let dest = temp_path.join(format!("subs.{}", ext));
        download_file(subs_input, &dest)?;
        Ok(dest)
    } else {
        let path = PathBuf::from(subs_input);
        if !path.exists() {
            bail!("Subtitle file does not exist: {}", subs_input);
        }
        Ok(path)
    }
}

fn find_adjacent_subtitle(video_path: &Path) -> Option<PathBuf> {
    let stem = video_path.file_stem()?;
    let parent = video_path.parent()?;

    // Check for common subtitle extensions
    for ext in &["srt", "ass", "ssa", "sub", "vtt"] {
        let sub_path = parent.join(format!("{}.{}", stem.to_string_lossy(), ext));
        if sub_path.exists() {
            println!("Found adjacent subtitle file: {}", sub_path.display());
            return Some(sub_path);
        }
    }

    None
}
