use anyhow::{Result, anyhow};
use reqwest::Client;
use std::path::{PathBuf};
use tokio::fs::{create_dir_all, File};
use tokio::io::AsyncWriteExt;
use crate::layers::PaperMetadata;

pub struct Downloader {
    client: Client,
    base_dir: PathBuf,
}

impl Downloader {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            client: Client::new(),
            base_dir: base_dir.into(),
        }
    }

    pub async fn download_paper(&self, paper: &PaperMetadata) -> Result<PathBuf> {
        if !paper.is_oa {
            return Err(anyhow!("Paper is not Open Access, skipping download."));
        }

        let pdf_url = paper.pdf_url.as_ref()
            .ok_or_else(|| anyhow!("No PDF URL found for paper despite OA status."))?;

        let raw_id = paper.doi.as_ref()
            .or(paper.arxiv_id.as_ref())
            .or(paper.semantic_scholar_id.as_ref())
            .map(|s| s.as_str())
            .unwrap_or("unknown_id");

        // Sanitize ID: remove scheme, replace non-alphanumeric chars
        let paper_id = raw_id.replace("http://", "")
                             .replace("https://", "")
                             .replace(|c: char| !c.is_alphanumeric() && c != '.' && c != '-', "_");

        // Download PDF
        tracing::info!("Downloading PDF from: {}", pdf_url);
        let mut response = self.client.get(pdf_url).send().await?;
        
        if !response.status().is_success() {
            let err = format!("Failed to download PDF: {}", response.status());
            tracing::error!("{}", err);
            return Err(anyhow!(err));
        }

        // Only create directory if request was successful
        let target_dir = self.base_dir.join(&paper_id);
        create_dir_all(&target_dir).await?;
        
        let pdf_path = target_dir.join("paper.pdf");
        let mut file = File::create(&pdf_path).await?;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk).await?;
        }

        // Save Metadata
        let metadata_path = target_dir.join("metadata.json");
        tracing::info!("Saving metadata to: {:?}", metadata_path);
        let metadata_json = serde_json::to_string_pretty(paper)?;
        let mut meta_file = File::create(&metadata_path).await?;
        meta_file.write_all(metadata_json.as_bytes()).await?;

        Ok(target_dir)
    }
}
