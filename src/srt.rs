use anyhow::{bail, Context, Result};
use regex::Regex;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct SubtitleEntry {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

pub fn parse_srt(path: &Path) -> Result<Vec<SubtitleEntry>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read subtitle file: {}", path.display()))?;

    let mut entries = Vec::new();
    let blocks: Vec<&str> = content.split("\n\n").collect();

    // SRT timestamp format: 00:01:23,456 --> 00:01:25,789
    let time_re = Regex::new(r"(\d{2}):(\d{2}):(\d{2})[,.](\d{3})\s*-->\s*(\d{2}):(\d{2}):(\d{2})[,.](\d{3})").unwrap();

    for block in blocks {
        let lines: Vec<&str> = block.lines().collect();
        if lines.len() < 3 {
            continue;
        }

        // Find the timestamp line (usually line 2, but be flexible)
        let mut timestamp_line = None;
        let mut text_start = 0;

        for (i, line) in lines.iter().enumerate() {
            if time_re.is_match(line) {
                timestamp_line = Some(*line);
                text_start = i + 1;
                break;
            }
        }

        let Some(ts_line) = timestamp_line else {
            continue;
        };

        let Some(caps) = time_re.captures(ts_line) else {
            continue;
        };

        let start = parse_srt_time(&caps[1], &caps[2], &caps[3], &caps[4]);
        let end = parse_srt_time(&caps[5], &caps[6], &caps[7], &caps[8]);

        // Join remaining lines as text, strip HTML tags
        let text: String = lines[text_start..]
            .join(" ")
            .replace("<i>", "")
            .replace("</i>", "")
            .replace("<b>", "")
            .replace("</b>", "")
            .replace("<u>", "")
            .replace("</u>", "")
            .trim()
            .to_string();

        if !text.is_empty() {
            entries.push(SubtitleEntry { start, end, text });
        }
    }

    Ok(entries)
}

fn parse_srt_time(hours: &str, mins: &str, secs: &str, millis: &str) -> f64 {
    let h: f64 = hours.parse().unwrap_or(0.0);
    let m: f64 = mins.parse().unwrap_or(0.0);
    let s: f64 = secs.parse().unwrap_or(0.0);
    let ms: f64 = millis.parse().unwrap_or(0.0);

    h * 3600.0 + m * 60.0 + s + ms / 1000.0
}

/// Find a subtitle entry containing the given text (case-insensitive fuzzy match)
pub fn find_dialogue<'a>(entries: &'a [SubtitleEntry], query: &str) -> Result<&'a SubtitleEntry> {
    let query_lower = query.to_lowercase();
    let query_words: Vec<&str> = query_lower.split_whitespace().collect();

    // First try: exact substring match
    for entry in entries {
        if entry.text.to_lowercase().contains(&query_lower) {
            return Ok(entry);
        }
    }

    // Second try: all words present in order (handles line breaks in subs)
    for entry in entries {
        let text_lower = entry.text.to_lowercase();
        let mut last_pos = 0;
        let mut all_found = true;

        for word in &query_words {
            if let Some(pos) = text_lower[last_pos..].find(word) {
                last_pos += pos + word.len();
            } else {
                all_found = false;
                break;
            }
        }

        if all_found {
            return Ok(entry);
        }
    }

    // Third try: fuzzy - most words present
    let mut best_match: Option<(&SubtitleEntry, usize)> = None;

    for entry in entries {
        let text_lower = entry.text.to_lowercase();
        let matches = query_words
            .iter()
            .filter(|w| text_lower.contains(*w))
            .count();

        if matches > 0 {
            if let Some((_, best_count)) = best_match {
                if matches > best_count {
                    best_match = Some((entry, matches));
                }
            } else {
                best_match = Some((entry, matches));
            }
        }
    }

    if let Some((entry, matches)) = best_match {
        if matches >= query_words.len() / 2 {
            return Ok(entry);
        }
    }

    bail!("Could not find dialogue: \"{}\"", query)
}
