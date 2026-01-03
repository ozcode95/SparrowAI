use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::collections::HashMap;
use reqwest;
use regex::Regex;
use once_cell::sync::Lazy;

const SKILLS_JSON_URL: &str = "https://raw.githubusercontent.com/intellectronica/awesome-skills/main/skills.json";
const SKILL_MARKDOWN: &str = "SKILL.md";

// Regex to parse YAML front matter in SKILL.md
static FRONT_MATTER_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^---\s*\n(.*?)\n---\s*\n(.*)").unwrap()
});

/// Deserialize null as empty string
fn deserialize_null_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

/// Metadata extracted from SKILL.md front matter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    #[serde(rename = "allowed-tools")]
    pub allowed_tools: Vec<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Represents a skill from the marketplace
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub title: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub description: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub author: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub author_url: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub author_github: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub license: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub license_url: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub skill_url: String,
    #[serde(default, deserialize_with = "deserialize_null_default")]
    pub skill_download_url: String,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Marketplace data structure
#[derive(Debug, Serialize, Deserialize)]
pub struct SkillsMarketplace {
    pub skills: Vec<Skill>,
}

/// Local skill metadata with installation status and parsed content
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledSkill {
    pub skill: Skill,
    pub installed_at: String,
    pub local_path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SkillMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    #[serde(default = "default_slug")]
    pub slug: String,
    #[serde(default)]
    pub resources: Vec<String>,
}

/// Default slug generator for backward compatibility
fn default_slug() -> String {
    String::new()
}

/// Convert skill name to slug (lowercase, hyphenated)
pub fn slugify(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-")
}

/// Parse SKILL.md file to extract metadata and instructions
pub fn parse_skill_md(skill_md_path: &Path) -> Result<(SkillMetadata, String)> {
    let content = fs::read_to_string(skill_md_path)
        .context("Failed to read SKILL.md")?;
    
    // Parse front matter and body
    let captures = FRONT_MATTER_PATTERN.captures(&content)
        .context("SKILL.md must begin with YAML front matter delimited by '---'")?;
    
    let front_matter = captures.get(1)
        .context("Failed to extract front matter")?.
        as_str();
    let body = captures.get(2)
        .context("Failed to extract body")?.
        as_str();
    
    // Parse YAML front matter
    let mut data: HashMap<String, serde_json::Value> = serde_yaml::from_str(front_matter)
        .context("Failed to parse YAML front matter")?;
    
    // Extract required fields
    let name = data.get("name")
        .and_then(|v| v.as_str())
        .context("Missing 'name' in front matter")?.
        to_string();
    
    let description = data.get("description")
        .and_then(|v| v.as_str())
        .context("Missing 'description' in front matter")?.
        to_string();
    
    // Extract optional fields
    let license = data.get("license")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    
    // Parse allowed-tools (can be string with commas or array)
    let allowed_tools = if let Some(tools) = data.get("allowed-tools").or_else(|| data.get("allowed_tools")) {
        if let Some(s) = tools.as_str() {
            s.split(',')
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .collect()
        } else if let Some(arr) = tools.as_array() {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.to_string())
                .collect()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    
    // Remove standard fields to get extra fields
    data.remove("name");
    data.remove("description");
    data.remove("license");
    data.remove("allowed-tools");
    data.remove("allowed_tools");
    
    let metadata = SkillMetadata {
        name,
        description,
        license,
        allowed_tools,
        extra: data,
    };
    
    Ok((metadata, body.trim().to_string()))
}

/// Get the skills directory path
pub fn get_skills_dir() -> Result<PathBuf> {
    let home_dir = dirs::home_dir().context("Failed to get home directory")?;
    Ok(home_dir.join(".sparrow").join("skills"))
}

/// Ensure skills directory exists
pub fn ensure_skills_dir() -> Result<PathBuf> {
    let skills_dir = get_skills_dir()?;
    fs::create_dir_all(&skills_dir)
        .context("Failed to create skills directory")?;
    Ok(skills_dir)
}

/// Get the marketplace cache file path
fn get_marketplace_cache_path() -> Result<PathBuf> {
    let skills_dir = get_skills_dir()?;
    Ok(skills_dir.join("marketplace_cache.json"))
}

/// Fetch the skills marketplace JSON from GitHub with caching
pub async fn fetch_marketplace() -> Result<SkillsMarketplace> {
    let cache_path = get_marketplace_cache_path()?;
    
    // Check if cache exists and is less than 1 hour old
    if cache_path.exists() {
        if let Ok(metadata) = fs::metadata(&cache_path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(elapsed) = modified.elapsed() {
                    // Use cache if less than 1 hour old
                    if elapsed.as_secs() < 3600 {
                        if let Ok(cached_data) = fs::read_to_string(&cache_path) {
                            match serde_json::from_str::<SkillsMarketplace>(&cached_data) {
                                Ok(marketplace) => {
                                    println!("Loaded {} skills from cache", marketplace.skills.len());
                                    return Ok(marketplace);
                                },
                                Err(e) => {
                                    eprintln!("Failed to parse cached data: {}", e);
                                    // Continue to fetch fresh data
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    
    // Fetch from GitHub
    println!("Fetching skills from GitHub...");
    let response = reqwest::get(SKILLS_JSON_URL)
        .await
        .context("Failed to fetch skills marketplace")?;
    
    let status = response.status();
    println!("GitHub response status: {}", status);
    
    let text = response.text().await.context("Failed to read response text")?;
    println!("Response length: {} bytes", text.len());
    
    // Parse the JSON with lenient error handling
    let marketplace: SkillsMarketplace = serde_json::from_str(&text)
        .map_err(|e| {
            eprintln!("JSON parsing error: {}", e);
            eprintln!("Response text preview: {}", &text[..text.len().min(500)]);
            anyhow::anyhow!("JSON parse error: {} (response preview: {})", e, &text[..text.len().min(200)])
        })?;
    
    println!("Successfully parsed {} skills from marketplace", marketplace.skills.len());
    
    // Save to cache
    let _ = ensure_skills_dir();
    let _ = fs::write(&cache_path, &text);
    
    Ok(marketplace)
}

/// Force refresh the marketplace cache
pub async fn refresh_marketplace() -> Result<SkillsMarketplace> {
    // Delete cache if it exists
    if let Ok(cache_path) = get_marketplace_cache_path() {
        let _ = fs::remove_file(cache_path);
    }
    fetch_marketplace().await
}

/// Create default metadata from marketplace skill data
fn create_default_metadata(skill: &Skill) -> SkillMetadata {
    SkillMetadata {
        name: if skill.title.is_empty() { skill.id.clone() } else { skill.title.clone() },
        description: skill.description.clone(),
        license: if skill.license.is_empty() { None } else { Some(skill.license.clone()) },
        allowed_tools: Vec::new(),
        extra: HashMap::new(),
    }
}

/// Scan skill directory for resource files
fn scan_skill_resources(skill_dir: &Path) -> Vec<String> {
    let mut resources = Vec::new();
    if let Ok(entries) = fs::read_dir(skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            
            if file_name != SKILL_MARKDOWN && file_name != "_metadata.json" && path.is_file() {
                resources.push(file_name.to_string());
            }
        }
    }
    resources
}

/// Download a skill from its repository
pub async fn download_skill(skill: &Skill) -> Result<PathBuf> {
    let skills_dir = ensure_skills_dir()?;
    let skill_dir = skills_dir.join(&skill.id);
    
    // Create skill directory
    fs::create_dir_all(&skill_dir)
        .context("Failed to create skill directory")?;
    
    let skill_url = &skill.skill_download_url;
    
    // Check if this is a Gist URL
    if skill_url.contains("gist.github.com") {
        download_from_gist(&skill_url, &skill_dir, skill).await?;
    } else {
        download_from_github_repo(&skill_url, &skill_dir, skill).await?;
    }
    
    Ok(skill_dir)
}

/// Download a skill from a GitHub Gist
async fn download_from_gist(gist_url: &str, skill_dir: &Path, skill: &Skill) -> Result<()> {
    let client = reqwest::Client::new();
    
    // Extract gist ID from URL
    // Format: https://gist.github.com/{user}/{gist_id}
    // or: https://gist.github.com/{user}/{gist_id}/raw/{commit}/{filename}
    let parts: Vec<&str> = gist_url.split('/').collect();
    let gist_id = if parts.len() >= 5 {
        parts[4]
    } else {
        anyhow::bail!("Invalid Gist URL format");
    };
    
    // Use GitHub API to get gist details
    let api_url = format!("https://api.github.com/gists/{}", gist_id);
    let response = client
        .get(&api_url)
        .header("User-Agent", "SparrowAI")
        .send()
        .await
        .context("Failed to fetch gist")?;
    
    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch gist: HTTP {}", response.status());
    }
    
    let gist: GistResponse = response
        .json()
        .await
        .context("Failed to parse Gist API response")?;
    
    // Download all files from the gist
    for (filename, file_info) in gist.files {
        if let Some(raw_url) = file_info.raw_url {
            let file_path = skill_dir.join(&filename);
            let content = client
                .get(&raw_url)
                .header("User-Agent", "SparrowAI")
                .send()
                .await?
                .bytes()
                .await?;
            
            fs::write(file_path, content)?;
        }
    }
    
    // Parse SKILL.md if it exists, otherwise create default metadata from marketplace data
    let skill_md_path = skill_dir.join(SKILL_MARKDOWN);
    let (metadata, instructions) = if skill_md_path.exists() {
        match parse_skill_md(&skill_md_path) {
            Ok((meta, instr)) => (Some(meta), Some(instr)),
            Err(e) => {
                eprintln!("Warning: Failed to parse SKILL.md for {}: {}. Using marketplace metadata.", skill.id, e);
                // Create default metadata from marketplace skill data
                let default_meta = SkillMetadata {
                    name: if skill.title.is_empty() { skill.id.clone() } else { skill.title.clone() },
                    description: skill.description.clone(),
                    license: if skill.license.is_empty() { None } else { Some(skill.license.clone()) },
                    allowed_tools: Vec::new(),
                    extra: HashMap::new(),
                };
                (Some(default_meta), None)
            }
        }
    } else {
        eprintln!("Warning: No SKILL.md found for {}. Using marketplace metadata.", skill.id);
        // Create default metadata from marketplace skill data
        let default_meta = SkillMetadata {
            name: if skill.title.is_empty() { skill.id.clone() } else { skill.title.clone() },
            description: skill.description.clone(),
            license: if skill.license.is_empty() { None } else { Some(skill.license.clone()) },
            allowed_tools: Vec::new(),
            extra: HashMap::new(),
        };
        (Some(default_meta), None)
    };
    
    // Collect resource files
    let mut resources = Vec::new();
    if let Ok(entries) = fs::read_dir(&skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            
            if file_name != SKILL_MARKDOWN && file_name != "_metadata.json" && path.is_file() {
                resources.push(file_name.to_string());
            }
        }
    }
    
    let slug = metadata.as_ref()
        .map(|m| slugify(&m.name))
        .unwrap_or_else(|| slugify(&skill.id));
    
    // Save metadata
    let installed_skill = InstalledSkill {
        skill: skill.clone(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        local_path: skill_dir.to_path_buf(),
        metadata,
        instructions,
        slug,
        resources,
    };
    
    let metadata_path = skill_dir.join("_metadata.json");
    let metadata_json = serde_json::to_string_pretty(&installed_skill)?;
    fs::write(metadata_path, metadata_json)
        .context("Failed to write skill metadata")?;
    
    Ok(())
}

/// Download a skill from a GitHub repository
async fn download_from_github_repo(skill_url: &str, skill_dir: &Path, skill: &Skill) -> Result<()> {
    // Parse the GitHub URL to get the download URL for the directory
    // Format: https://github.com/{owner}/{repo}/tree/{branch}/{path}
    
    // Convert GitHub tree URL to raw API URL
    // Example: https://github.com/anthropics/skills/tree/main/mcp-builder
    // to: https://api.github.com/repos/anthropics/skills/contents/mcp-builder?ref=main
    
    let parts: Vec<&str> = skill_url.split('/').collect();
    if parts.len() < 7 || parts[2] != "github.com" {
        anyhow::bail!("Invalid GitHub URL format");
    }
    
    let owner = parts[3];
    let repo = parts[4];
    let branch = if parts[5] == "tree" { parts[6] } else { "main" };
    let path = parts[7..].join("/");
    
    let api_url = format!(
        "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
        owner, repo, path, branch
    );
    
    // Fetch directory contents from GitHub API
    let client = reqwest::Client::new();
    let response = client
        .get(&api_url)
        .header("User-Agent", "SparrowAI")
        .send()
        .await
        .context("Failed to fetch skill contents")?;
    
    if !response.status().is_success() {
        anyhow::bail!("Failed to fetch skill: HTTP {}", response.status());
    }
    
    let contents: Vec<GitHubContent> = response
        .json()
        .await
        .context("Failed to parse GitHub API response")?;
    
    // Download all files
    download_directory_contents(&client, &contents, &skill_dir, owner, repo, branch).await?;
    
    // Parse SKILL.md if it exists, otherwise create default metadata from marketplace data
    let skill_md_path = skill_dir.join(SKILL_MARKDOWN);
    let (metadata, instructions) = if skill_md_path.exists() {
        match parse_skill_md(&skill_md_path) {
            Ok((meta, instr)) => (Some(meta), Some(instr)),
            Err(e) => {
                eprintln!("Warning: Failed to parse SKILL.md for {}: {}. Using marketplace metadata.", skill.id, e);
                // Create default metadata from marketplace skill data
                let default_meta = SkillMetadata {
                    name: if skill.title.is_empty() { skill.id.clone() } else { skill.title.clone() },
                    description: skill.description.clone(),
                    license: if skill.license.is_empty() { None } else { Some(skill.license.clone()) },
                    allowed_tools: Vec::new(),
                    extra: HashMap::new(),
                };
                (Some(default_meta), None)
            }
        }
    } else {
        eprintln!("Warning: No SKILL.md found for {}. Using marketplace metadata.", skill.id);
        // Create default metadata from marketplace skill data
        let default_meta = SkillMetadata {
            name: if skill.title.is_empty() { skill.id.clone() } else { skill.title.clone() },
            description: skill.description.clone(),
            license: if skill.license.is_empty() { None } else { Some(skill.license.clone()) },
            allowed_tools: Vec::new(),
            extra: HashMap::new(),
        };
        (Some(default_meta), None)
    };
    
    // Collect resource files
    let mut resources = Vec::new();
    if let Ok(entries) = fs::read_dir(&skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            
            if file_name != SKILL_MARKDOWN && file_name != "_metadata.json" && path.is_file() {
                resources.push(file_name.to_string());
            }
        }
    }
    
    let slug = metadata.as_ref()
        .map(|m| slugify(&m.name))
        .unwrap_or_else(|| slugify(&skill.id));
    
    // Save metadata
    let installed_skill = InstalledSkill {
        skill: skill.clone(),
        installed_at: chrono::Utc::now().to_rfc3339(),
        local_path: skill_dir.to_path_buf(),
        metadata,
        instructions,
        slug,
        resources,
    };
    
    let metadata_path = skill_dir.join("_metadata.json");
    let metadata_json = serde_json::to_string_pretty(&installed_skill)?;
    fs::write(metadata_path, metadata_json)
        .context("Failed to write skill metadata")?;
    
    Ok(())
}

/// GitHub API content structure
#[derive(Debug, Deserialize)]
struct GitHubContent {
    name: String,
    path: String,
    #[serde(rename = "type")]
    content_type: String,
    download_url: Option<String>,
    url: String,
}

/// GitHub Gist API response structure
#[derive(Debug, Deserialize)]
struct GistResponse {
    files: std::collections::HashMap<String, GistFile>,
}

#[derive(Debug, Deserialize)]
struct GistFile {
    filename: String,
    raw_url: Option<String>,
}

/// Recursively download directory contents from GitHub
async fn download_directory_contents(
    client: &reqwest::Client,
    contents: &[GitHubContent],
    base_dir: &Path,
    owner: &str,
    repo: &str,
    branch: &str,
) -> Result<()> {
    for item in contents {
        match item.content_type.as_str() {
            "file" => {
                if let Some(download_url) = &item.download_url {
                    let file_path = base_dir.join(&item.name);
                    let content = client
                        .get(download_url)
                        .header("User-Agent", "SparrowAI")
                        .send()
                        .await?
                        .bytes()
                        .await?;
                    
                    fs::write(file_path, content)?;
                }
            }
            "dir" => {
                let dir_path = base_dir.join(&item.name);
                fs::create_dir_all(&dir_path)?;
                
                // Fetch subdirectory contents
                let subdir_url = format!(
                    "https://api.github.com/repos/{}/{}/contents/{}?ref={}",
                    owner, repo, item.path, branch
                );
                
                let response = client
                    .get(&subdir_url)
                    .header("User-Agent", "SparrowAI")
                    .send()
                    .await?;
                
                let subcontents: Vec<GitHubContent> = response.json().await?;
                Box::pin(download_directory_contents(client, &subcontents, &dir_path, owner, repo, branch)).await?;
            }
            _ => {}
        }
    }
    Ok(())
}

/// Delete a skill
pub async fn delete_skill(skill_id: &str) -> Result<()> {
    let skills_dir = get_skills_dir()?;
    let skill_dir = skills_dir.join(skill_id);
    
    if skill_dir.exists() {
        fs::remove_dir_all(&skill_dir)
            .context("Failed to delete skill directory")?;
    }
    
    Ok(())
}

/// Get list of installed skills
pub fn get_installed_skills() -> Result<Vec<InstalledSkill>> {
    let skills_dir = match get_skills_dir() {
        Ok(dir) => dir,
        Err(_) => return Ok(vec![]),
    };
    
    if !skills_dir.exists() {
        return Ok(vec![]);
    }
    
    let mut installed = Vec::new();
    
    for entry in fs::read_dir(skills_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.is_dir() {
            let metadata_path = path.join("_metadata.json");
            if metadata_path.exists() {
                let content = fs::read_to_string(&metadata_path)?;
                if let Ok(mut skill) = serde_json::from_str::<InstalledSkill>(&content) {
                    // Fix up skills loaded from old format
                    if skill.slug.is_empty() {
                        skill.slug = slugify(&skill.skill.id);
                    }
                    
                    // Create default metadata if missing
                    if skill.metadata.is_none() {
                        skill.metadata = Some(create_default_metadata(&skill.skill));
                    }
                    
                    // Scan for resources if empty
                    if skill.resources.is_empty() {
                        skill.resources = scan_skill_resources(&path);
                    }
                    
                    // Parse instructions if missing
                    if skill.instructions.is_none() {
                        let skill_md_path = path.join("SKILL.md");
                        if skill_md_path.exists() {
                            if let Ok((_, instructions_text)) = parse_skill_md(&skill_md_path) {
                                skill.instructions = Some(instructions_text);
                            }
                        }
                    }
                    
                    installed.push(skill);
                }
            }
        }
    }
    
    Ok(installed)
}

/// Check if a skill is installed
pub fn is_skill_installed(skill_id: &str) -> bool {
    if let Ok(skills_dir) = get_skills_dir() {
        let skill_dir = skills_dir.join(skill_id);
        skill_dir.exists() && skill_dir.join("_metadata.json").exists()
    } else {
        false
    }
}
