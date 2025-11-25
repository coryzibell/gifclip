use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ToolSource {
    System,
    Managed,
}

impl Default for ToolSource {
    fn default() -> Self {
        Self::System
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub tool_source: ToolSource,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_path = Self::config_path()?;

        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config from {}", config_path.display()))?;

        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {}", config_path.display()))
    }

    pub fn save(&self) -> Result<()> {
        let config_path = Self::config_path()?;

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create config directory {}", parent.display()))?;
        }

        let content = toml::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(&config_path, content)
            .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

        Ok(())
    }

    pub fn config_dir() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Could not find home directory")?;
        Ok(home.join(".gifclip"))
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("settings.toml"))
    }

    pub fn tools_dir() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("tools"))
    }

    pub fn yt_dlp_path(&self) -> Result<PathBuf> {
        match self.tool_source {
            ToolSource::System => {
                which::which("yt-dlp").context("yt-dlp not found in PATH")
            }
            ToolSource::Managed => {
                let tools_dir = Self::tools_dir()?;
                #[cfg(windows)]
                let name = "yt-dlp.exe";
                #[cfg(not(windows))]
                let name = "yt-dlp";
                Ok(tools_dir.join(name))
            }
        }
    }

    pub fn ffmpeg_path(&self) -> Result<PathBuf> {
        match self.tool_source {
            ToolSource::System => {
                which::which("ffmpeg").context("ffmpeg not found in PATH")
            }
            ToolSource::Managed => {
                let tools_dir = Self::tools_dir()?;
                #[cfg(windows)]
                let name = "ffmpeg.exe";
                #[cfg(not(windows))]
                let name = "ffmpeg";
                Ok(tools_dir.join(name))
            }
        }
    }
}
