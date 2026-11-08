use anyhow::{Result, anyhow, bail};
use reqwest::{Client, Url};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs::{self, create_dir_all, File};
use tokio::io::AsyncWriteExt;
use crate::layers::PaperMetadata;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use chrono::Utc;
use url::Host;

const MAX_REDIRECTS: usize = 5;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ManifestEntry {
    title: String,
    first_author: String,
    year: Option<u32>,
    id: String,
    relative_path: String,
    downloaded_at: String,
}

pub struct Downloader {
    client: Client,
    base_dir: PathBuf,
    pub max_bytes: u64,
    /// Off by default (SSRF guard); tests enable it to reach a local mock server.
    pub allow_private_hosts: bool,
}

impl Downloader {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            client: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(120))
                .build()
                .expect("failed to build HTTP client"),
            base_dir: base_dir.into(),
            max_bytes: 100 * 1024 * 1024,
            allow_private_hosts: false,
        }
    }

    pub async fn download_paper(&self, paper: &PaperMetadata) -> Result<PathBuf> {
        if !paper.is_oa {
            return Err(anyhow!("Paper is not Open Access, skipping download."));
        }

        let pdf_url = paper.pdf_url.as_ref()
            .ok_or_else(|| anyhow!("No PDF URL found for paper despite OA status."))?;

        let paper_id = Self::storage_id(paper);

        tracing::info!("Downloading PDF from: {}", pdf_url);
        let response = self.get_following_redirects(pdf_url).await?;
        if response.content_length().is_some_and(|n| n > self.max_bytes) {
            bail!("PDF exceeds {} bytes", self.max_bytes);
        }

        create_dir_all(&self.base_dir).await?;
        let part_path = self.base_dir.join(format!(".{}.part", paper_id));
        if let Err(e) = self.stream_pdf(response, &part_path).await {
            let _ = fs::remove_file(&part_path).await;
            return Err(e);
        }

        let target_dir = self.base_dir.join(&paper_id);
        create_dir_all(&target_dir).await?;
        let pdf_path = target_dir.join("paper.pdf");
        fs::rename(&part_path, &pdf_path).await?;

        let metadata_path = target_dir.join("metadata.json");
        tracing::info!("Saving metadata to: {:?}", metadata_path);
        write_atomic(&metadata_path, serde_json::to_string_pretty(paper)?).await?;

        // Update Manifest
        self.update_manifest(paper, &paper_id, &pdf_path).await?;

        Ok(target_dir)
    }

    async fn get_following_redirects(&self, url: &str) -> Result<reqwest::Response> {
        let mut url = Url::parse(url)?;
        for _ in 0..=MAX_REDIRECTS {
            self.check_url(&url)?;
            let response = self.client.get(url.clone()).send().await?;
            if response.status().is_redirection() {
                let location = response.headers().get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| anyhow!("redirect without Location header"))?;
                url = url.join(location)?;
                continue;
            }
            if !response.status().is_success() {
                bail!("Failed to download PDF: {}", response.status());
            }
            return Ok(response);
        }
        bail!("too many redirects")
    }

    fn check_url(&self, url: &Url) -> Result<()> {
        if !matches!(url.scheme(), "http" | "https") {
            bail!("refusing non-HTTP URL: {}", url);
        }
        if self.allow_private_hosts {
            return Ok(());
        }
        let private_v4 = |ip: Ipv4Addr| ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast();
        let private = match url.host() {
            Some(Host::Domain(d)) => d.eq_ignore_ascii_case("localhost") || d.to_ascii_lowercase().ends_with(".localhost"),
            Some(Host::Ipv4(ip)) => private_v4(ip),
            Some(Host::Ipv6(ip)) => ip.is_loopback() || ip.is_unspecified()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
                || ip.to_ipv4_mapped().is_some_and(private_v4),
            None => true,
        };
        if private {
            bail!("refusing to download from private or local host: {}", url);
        }
        Ok(())
    }

    async fn stream_pdf(&self, mut response: reqwest::Response, path: &Path) -> Result<()> {
        let mut file = File::create(path).await?;
        let mut written: u64 = 0;
        let mut head: Vec<u8> = Vec::with_capacity(5);
        while let Some(chunk) = response.chunk().await? {
            written += chunk.len() as u64;
            if written > self.max_bytes {
                bail!("PDF exceeds {} bytes", self.max_bytes);
            }
            if head.len() < 5 {
                head.extend(chunk.iter().take(5 - head.len()));
                if head.len() == 5 && head != b"%PDF-" {
                    bail!("response is not a PDF");
                }
            }
            file.write_all(&chunk).await?;
        }
        if head.len() < 5 {
            bail!("response is not a PDF");
        }
        file.flush().await?;
        Ok(())
    }

    /// Directory name a paper is stored under: first of DOI / arXiv / Semantic Scholar / OpenAlex ID, sanitized.
    pub fn storage_id(paper: &PaperMetadata) -> String {
        let raw_id = paper.doi.as_ref()
            .or(paper.arxiv_id.as_ref())
            .or(paper.semantic_scholar_id.as_ref())
            .or(paper.open_alex_id.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("unknown_id");

        // Sanitize ID: remove scheme, replace non-alphanumeric chars
        raw_id.replace("http://", "")
              .replace("https://", "")
              .replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "_")
    }

    async fn update_manifest(&self, paper: &PaperMetadata, id: &str, pdf_path: &std::path::Path) -> Result<()> {
        let manifest_path = self.base_dir.join("manifest.json");
        let mut entries: Vec<ManifestEntry> = read_json_or_backup(&manifest_path).await?;

        let first_author = paper.authors.first().map(|s| s.as_str()).unwrap_or("Unknown").to_string();
        let relative_path = pdf_path.strip_prefix(&self.base_dir)
            .unwrap_or(pdf_path)
            .to_string_lossy()
            .into_owned();

        let new_entry = ManifestEntry {
            title: paper.title.clone(),
            first_author,
            year: paper.year,
            id: id.to_string(),
            relative_path,
            downloaded_at: Utc::now().to_rfc3339(),
        };

        // Remove existing entry with same ID if exists (update)
        entries.retain(|e| e.id != id);
        entries.push(new_entry);

        write_atomic(&manifest_path, serde_json::to_string_pretty(&entries)?).await?;
        tracing::info!("Updated manifest at: {:?}", manifest_path);

        Ok(())
    }

    pub async fn save_unavailable(&self, query: &crate::layers::DiscoveryQuery, papers: Vec<PaperMetadata>) -> Result<()> {
        if papers.is_empty() {
            return Ok(());
        }

        let path = self.base_dir.join("unavailable.json");
        let mut root: serde_json::Value = read_json_or_backup(&path).await?;

        // Determine nesting keys based on priority
        let mut keys = Vec::new();
        if let Some(u) = &query.university { keys.push(u.clone()); }
        if let Some(c) = &query.category { keys.push(c.clone()); }
        if let Some(a) = &query.author { keys.push(a.clone()); }
        if let Some(t) = &query.title { keys.push(t.clone()); }
        if let Some(t) = &query.topic { keys.push(t.clone()); }
        
        if keys.is_empty() {
            keys.push("General_Search".to_string());
        }

        // Navigate/Build the structure
        let mut current = &mut root;
        for (i, key) in keys.iter().enumerate() {
            // If we are at the last key, we want a List.
            // If not, we want an Object.
            let is_last = i == keys.len() - 1;

            if !current.is_object() {
                 // Should ideally not happen if structure matches, but safety reset if type mismatch
                 *current = serde_json::Value::Object(serde_json::Map::new());
            }
            
            if is_last {
                 // Initialize list if not present or not an array
                 if current.get(key).is_none() || !current[key].is_array() {
                     current[key] = serde_json::Value::Array(Vec::new());
                 }
                 // Now we add our papers to this array
                 if let Some(arr) = current[key].as_array_mut() {
                     for p in &papers {
                         // Simple check to avoid duplicates if possible, or just append
                         // Converting to Value to compare/insert
                         let p_val = serde_json::to_value(p)?;
                         // Check if already exists (simple O(N) check)
                         let exists = arr.iter().any(|existing| {
                             existing["title"] == p_val["title"] 
                             && existing["year"] == p_val["year"]
                         });
                         
                         if !exists {
                             arr.push(p_val);
                         }
                     }
                 }
            } else {
                // Intermediate node -> Ensure it exists as Object
                if current.get(key).is_none() {
                    current.as_object_mut().unwrap().insert(key.clone(), serde_json::Value::Object(serde_json::Map::new()));
                }
                current = current.get_mut(key).unwrap();
            }
        }

        write_atomic(&path, serde_json::to_string_pretty(&root)?).await?;
        tracing::info!("Saved {} unavailable papers to unavailable.json", papers.len());

        Ok(())
    }
}

async fn write_atomic(path: &Path, contents: String) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, contents).await?;
    fs::rename(&tmp, path).await?;
    Ok(())
}

/// Missing file -> default. Unparseable file -> renamed to `*.corrupt-<timestamp>` (never silently discarded) -> default.
async fn read_json_or_backup<T: DeserializeOwned + Default>(path: &Path) -> Result<T> {
    if !path.exists() {
        return Ok(T::default());
    }
    let content = fs::read_to_string(path).await?;
    match serde_json::from_str(&content) {
        Ok(value) => Ok(value),
        Err(e) => {
            let backup = path.with_extension(format!("json.corrupt-{}", Utc::now().format("%Y%m%dT%H%M%S%.f")));
            tracing::warn!("{:?} is not valid JSON ({}); moved to {:?}", path, e, backup);
            fs::rename(path, &backup).await?;
            Ok(T::default())
        }
    }
}
