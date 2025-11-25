use anyhow::{bail, Context, Result};
use dialoguer::Select;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
use std::fs::File;
#[cfg(target_os = "linux")]
use tar::Archive;

use crate::config::{Config, ToolSource};

pub fn run_setup() -> Result<Config> {
    println!("gifclip setup\n");

    let has_system_ytdlp = which::which("yt-dlp").is_ok();
    let has_system_ffmpeg = which::which("ffmpeg").is_ok();

    let choice = if has_system_ytdlp && has_system_ffmpeg {
        println!("Found system installations:");
        if has_system_ytdlp {
            println!("  yt-dlp: {}", which::which("yt-dlp").unwrap().display());
        }
        if has_system_ffmpeg {
            println!("  ffmpeg: {}", which::which("ffmpeg").unwrap().display());
        }
        println!();

        let options = &[
            "Use system tools (recommended if already installed)",
            "Download and manage tools in ~/.gifclip/tools",
        ];

        Select::new()
            .with_prompt("How would you like gifclip to access yt-dlp and ffmpeg?")
            .items(options)
            .default(0)
            .interact()
            .context("Failed to get user selection")?
    } else {
        println!("System tools not found:");
        if !has_system_ytdlp {
            println!("  yt-dlp: not found");
        }
        if !has_system_ffmpeg {
            println!("  ffmpeg: not found");
        }
        println!();

        let options = &[
            "Download and manage tools in ~/.gifclip/tools (recommended)",
            "I'll install them myself (use system PATH)",
        ];

        let choice = Select::new()
            .with_prompt("How would you like to proceed?")
            .items(options)
            .default(0)
            .interact()
            .context("Failed to get user selection")?;

        // Flip the choice since options are reversed for this case
        if choice == 0 { 1 } else { 0 }
    };

    let tool_source = if choice == 0 {
        ToolSource::System
    } else {
        ToolSource::Managed
    };

    let config = Config { tool_source };

    if config.tool_source == ToolSource::Managed {
        download_tools(&config)?;
    }

    config.save()?;
    println!("\nConfiguration saved to {}", Config::config_path()?.display());

    Ok(config)
}

pub fn ensure_setup() -> Result<Config> {
    let config = Config::load()?;

    // Check if tools are available
    let yt_dlp_ok = config.yt_dlp_path().is_ok_and(|p| p.exists());
    let ffmpeg_ok = config.ffmpeg_path().is_ok_and(|p| p.exists());

    if !yt_dlp_ok || !ffmpeg_ok {
        // Need setup
        if config.tool_source == ToolSource::Managed {
            // Tools should be managed but missing - redownload
            println!("Managed tools missing, downloading...");
            download_tools(&config)?;
            return Ok(config);
        }

        // No config or system tools missing - run interactive setup
        println!("gifclip requires yt-dlp and ffmpeg to work.\n");
        return run_setup();
    }

    Ok(config)
}

fn download_tools(_config: &Config) -> Result<()> {
    let tools_dir = Config::tools_dir()?;
    fs::create_dir_all(&tools_dir)
        .with_context(|| format!("Failed to create tools directory: {}", tools_dir.display()))?;

    println!("\nDownloading tools to {}...", tools_dir.display());

    download_ytdlp(&tools_dir)?;
    download_ffmpeg(&tools_dir)?;

    println!("Tools installed successfully!");

    Ok(())
}

fn download_ytdlp(tools_dir: &Path) -> Result<()> {
    print!("Downloading yt-dlp... ");
    io::stdout().flush()?;

    #[cfg(target_os = "linux")]
    let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp";
    #[cfg(target_os = "macos")]
    let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos";
    #[cfg(target_os = "windows")]
    let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
    #[cfg(target_os = "freebsd")]
    let url = "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp";
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "freebsd")))]
    bail!("Managed tool download is not supported on this platform. Please install yt-dlp manually.");

    #[cfg(windows)]
    let dest = tools_dir.join("yt-dlp.exe");
    #[cfg(not(windows))]
    let dest = tools_dir.join("yt-dlp");

    let response = reqwest::blocking::get(url)
        .context("Failed to download yt-dlp")?;

    if !response.status().is_success() {
        bail!("Failed to download yt-dlp: HTTP {}", response.status());
    }

    let bytes = response.bytes().context("Failed to read yt-dlp download")?;
    fs::write(&dest, &bytes)
        .with_context(|| format!("Failed to write yt-dlp to {}", dest.display()))?;

    #[cfg(unix)]
    {
        let mut perms = fs::metadata(&dest)?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&dest, perms)?;
    }

    println!("done");
    Ok(())
}

fn download_ffmpeg(tools_dir: &Path) -> Result<()> {
    print!("Downloading ffmpeg... ");
    io::stdout().flush()?;

    // Use ffmpeg-static builds from https://johnvansickle.com/ffmpeg/ (Linux)
    // or https://evermeet.cx/ffmpeg/ (macOS)
    // or https://www.gyan.dev/ffmpeg/builds/ (Windows)
    // FreeBSD and other platforms: no pre-built binaries available

    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        target_os = "macos",
        target_os = "windows"
    )))]
    {
        println!();
        bail!("Managed ffmpeg download is not supported on this platform. Please install ffmpeg manually.");
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    let url = "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-amd64-static.tar.xz";
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    let url = "https://johnvansickle.com/ffmpeg/releases/ffmpeg-release-arm64-static.tar.xz";
    #[cfg(target_os = "macos")]
    let url = "https://evermeet.cx/ffmpeg/getrelease/zip";
    #[cfg(target_os = "windows")]
    let url = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";

    #[cfg(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        target_os = "macos",
        target_os = "windows"
    ))]
    {
        let response = reqwest::blocking::get(url)
            .context("Failed to download ffmpeg")?;

        if !response.status().is_success() {
            bail!("Failed to download ffmpeg: HTTP {}", response.status());
        }

        let bytes = response.bytes().context("Failed to read ffmpeg download")?;

        #[cfg(target_os = "linux")]
        extract_ffmpeg_linux(&bytes, tools_dir)?;

        #[cfg(target_os = "macos")]
        extract_ffmpeg_macos(&bytes, tools_dir)?;

        #[cfg(target_os = "windows")]
        extract_ffmpeg_windows(&bytes, tools_dir)?;

        println!("done");
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn extract_ffmpeg_linux(bytes: &[u8], tools_dir: &Path) -> Result<()> {
    use std::io::Cursor;
    use xz2::read::XzDecoder;

    let cursor = Cursor::new(bytes);
    let xz = XzDecoder::new(cursor);
    let mut archive = Archive::new(xz);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?;

        // Look for the ffmpeg binary in the archive
        if let Some(name) = path.file_name() {
            if name == "ffmpeg" {
                let dest = tools_dir.join("ffmpeg");
                let mut file = File::create(&dest)?;
                io::copy(&mut entry, &mut file)?;

                let mut perms = fs::metadata(&dest)?.permissions();
                perms.set_mode(0o755);
                fs::set_permissions(&dest, perms)?;

                return Ok(());
            }
        }
    }

    bail!("ffmpeg binary not found in archive")
}

#[cfg(target_os = "macos")]
fn extract_ffmpeg_macos(bytes: &[u8], tools_dir: &Path) -> Result<()> {
    use std::io::Cursor;

    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name();

        if name == "ffmpeg" || name.ends_with("/ffmpeg") {
            let dest = tools_dir.join("ffmpeg");
            let mut outfile = File::create(&dest)?;
            io::copy(&mut file, &mut outfile)?;

            let mut perms = fs::metadata(&dest)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&dest, perms)?;

            return Ok(());
        }
    }

    bail!("ffmpeg binary not found in archive")
}

#[cfg(target_os = "windows")]
fn extract_ffmpeg_windows(bytes: &[u8], tools_dir: &Path) -> Result<()> {
    use std::io::Cursor;

    let cursor = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name();

        if name.ends_with("ffmpeg.exe") {
            let dest = tools_dir.join("ffmpeg.exe");
            let mut outfile = File::create(&dest)?;
            io::copy(&mut file, &mut outfile)?;
            return Ok(());
        }
    }

    bail!("ffmpeg.exe not found in archive")
}
