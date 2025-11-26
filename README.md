# gifclip

Create GIFs (or video clips) with burned-in subtitles from YouTube, local files, or direct URLs.

## Installation

### From crates.io

```bash
cargo install gifclip
```

### From source

```bash
git clone https://github.com/coryzibell/gifclip
cd gifclip
cargo install --path .
```

### Dependencies

gifclip requires **yt-dlp** and **ffmpeg**. On first run, gifclip will prompt you to either:

1. **Use system tools** - Use yt-dlp and ffmpeg from your PATH
2. **Managed tools** - Download and manage tools in `~/.gifclip/tools/`

You can reconfigure this at any time with `gifclip --setup`.

#### Installing dependencies manually

**macOS:**
```bash
brew install yt-dlp ffmpeg
```

**Debian/Ubuntu:**
```bash
sudo apt install yt-dlp ffmpeg
```

**Arch Linux:**
```bash
sudo pacman -S yt-dlp ffmpeg
```

**NixOS:**
```nix
environment.systemPackages = with pkgs; [ yt-dlp ffmpeg ];
```

## Usage

### Basic Usage

```bash
gifclip <INPUT> <START> <END>
```

Where `<INPUT>` can be:
- **YouTube URL** - Downloads via yt-dlp, auto-fetches subtitles
- **Local file** - Uses embedded subs or looks for matching `.srt` file
- **Direct URL** - Downloads video, extracts embedded subs if available

### Timestamp Mode

Clip a video using specific start and end timestamps:

```bash
# YouTube
gifclip "https://youtube.com/watch?v=..." 1:30 1:45

# Local file
gifclip movie.mp4 0:45 0:59

# Direct URL
gifclip "https://example.com/video.mp4" 0:10 0:20
```

Timestamps support multiple formats:
- `MM:SS` - minutes and seconds (e.g., `1:30`)
- `HH:MM:SS` - hours, minutes, seconds (e.g., `00:01:30`)
- Seconds as a number (e.g., `90`)

### Dialogue Mode

Search subtitles for dialogue and clip around it automatically:

```bash
# Single quote - clips the subtitle entry with 2s padding
gifclip "URL" --from "I'll be back"

# Dialogue range - clips from first quote to second with 0.5s padding
gifclip "URL" --from "Here's looking" --to "kid"
```

The dialogue search is fuzzy and case-insensitive, so partial matches work.

### Custom Padding

Control how much video appears before/after the dialogue:

```bash
# Symmetric padding (before and after)
gifclip "URL" --from "quote" --pad 3

# Asymmetric padding
gifclip "URL" --from "quote" --pad-before 1 --pad-after 5
```

### Subtitle Handling

gifclip automatically finds subtitles based on input type:

| Input Type | Subtitle Search Order |
|------------|----------------------|
| YouTube | Fetches from YouTube (auto + uploaded) |
| Local file | 1. Embedded subs, 2. Adjacent `.srt`/`.ass`/etc. |
| Direct URL | Embedded subs only |

Override with `--subs` or disable with `--no-subs`:

```bash
# Use specific subtitle file
gifclip movie.mp4 1:30 1:45 --subs custom.srt

# Use subtitle URL
gifclip movie.mp4 1:30 1:45 --subs "https://example.com/subs.srt"

# Skip subtitles entirely
gifclip movie.mp4 1:30 1:45 --no-subs
```

### Custom Text Overlay

Add your own caption instead of using subtitles:

```bash
gifclip movie.mp4 1:30 1:45 --text "Your caption here"
```

The text is displayed centered at the bottom with a black outline for visibility.

### Output Formats

```bash
# GIF (default)
gifclip "URL" 1:30 1:45

# WebM (smaller file, good quality)
gifclip "URL" 1:30 1:45 -f webm

# MP4 (most compatible)
gifclip "URL" 1:30 1:45 -f mp4
```

### Options

| Option | Description | Default |
|--------|-------------|---------|
| `--subs <FILE_OR_URL>` | External subtitle file or URL | Auto-detect |
| `--text <STRING>` | Custom text overlay (replaces subtitles) | - |
| `-o, --output <FILE>` | Output filename | Auto-generated |
| `-f, --format <FMT>` | Output format: `gif`, `webm`, `mp4` | `gif` |
| `-w, --width <PX>` | Width in pixels (height scales proportionally) | `480` |
| `--fps <N>` | Frames per second | `15` |
| `--lang <CODE>` | Subtitle language code (YouTube only) | `en` |
| `--no-subs` | Skip subtitles | false |
| `-q, --quality <1-100>` | Quality (higher = better, larger file) | `80` |

### Examples

```bash
# Basic GIF with subtitles from YouTube
gifclip "https://youtube.com/watch?v=abc123" 0:45 0:59

# Higher quality MP4, 720px wide
gifclip "URL" 1:30 2:00 -f mp4 -w 720 -q 95

# GIF of a famous quote with extra time after
gifclip "URL" --from "Frankly my dear" --pad-after 3

# Clip a conversation between two lines
gifclip "URL" --from "What is the Matrix?" --to "No one can be told"

# Local file with auto-detected subtitles
gifclip movie.mkv 0:30 0:45

# Local file with custom subs
gifclip movie.mp4 --subs movie.srt 1:00 1:30

# French subtitles from YouTube
gifclip "URL" 1:00 1:15 --lang fr
```

## Configuration

Configuration is stored in `~/.gifclip/settings.toml`:

```toml
tool_source = "system"  # or "managed"
```

Run `gifclip --setup` to reconfigure.

## License

MIT
