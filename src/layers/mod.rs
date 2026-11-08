use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PaperMetadata {
    #[serde(default)]
    pub id: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<u32>,
    pub doi: Option<String>,
    pub arxiv_id: Option<String>,
    pub semantic_scholar_id: Option<String>,
    pub open_alex_id: Option<String>,
    pub venue: Option<String>,
    pub abstract_text: Option<String>,
    pub pdf_url: Option<String>,
    pub is_oa: bool,
    pub categories: Vec<String>,
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SourceStatus {
    pub source: String,
    pub ok: bool,
    pub result_count: usize,
    pub latency_ms: u64,
    pub http_status: Option<u16>,
    pub error: Option<String>,
}

pub const SCHEMA_VERSION: u32 = 1;

/// Machine-readable output of `--json`. Bump `SCHEMA_VERSION` on breaking changes.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LiteratureResult {
    pub schema_version: u32,
    pub papers: Vec<PaperMetadata>,
    pub source_statuses: Vec<SourceStatus>,
    #[serde(default)]
    pub downloads: Vec<DownloadRecord>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DownloadRecord {
    pub id: String,
    pub path: Option<String>,
    pub error: Option<String>,
}

pub struct DiscoveryQuery {
    pub title: Option<String>,
    pub author: Option<String>,
    #[allow(dead_code)]
    pub university: Option<String>,
    pub category: Option<String>,
    /// Free-text topic search; not subject to title matching.
    pub topic: Option<String>,
    pub limit: usize,
}

pub mod discovery;
pub mod resolution;
pub mod download;
pub mod legality;
pub mod identity;
