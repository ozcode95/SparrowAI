//! Minimal GitHub releases API client. We only need the latest release plus
//! its asset list — no auth, no pagination, no caching beyond what the OS
//! HTTP cache provides.

use anyhow::{anyhow, Result};
use serde::Deserialize;

const UPSTREAM_OWNER: &str = "openvinotoolkit";
const UPSTREAM_REPO: &str = "model_server";

#[derive(Debug, Deserialize, Clone)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub published_at: Option<String>,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
}

impl Release {
    /// Returns the first asset whose name passes `predicate`. We use a
    /// predicate rather than a fixed name so we can support Windows
    /// (`python_off`) today and add Linux / macOS variants later without
    /// changing the API surface.
    pub fn find_asset<F: Fn(&str) -> bool>(&self, predicate: F) -> Option<&Asset> {
        self.assets.iter().find(|a| predicate(&a.name))
    }
}

pub async fn latest_release(http: &reqwest::Client) -> Result<Release> {
    let url = format!(
        "https://api.github.com/repos/{UPSTREAM_OWNER}/{UPSTREAM_REPO}/releases/latest"
    );
    let resp = http
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?
        .error_for_status()?;
    let release: Release = resp.json().await?;
    if release.assets.is_empty() {
        return Err(anyhow!("github: release {} has no assets", release.tag_name));
    }
    Ok(release)
}

/// Default matcher for the asset we install on Windows: the python-free
/// zip distribution. Matches case-insensitively on substring + extension.
pub fn match_windows_python_off(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("windows") && n.contains("python_off") && n.ends_with(".zip")
}
