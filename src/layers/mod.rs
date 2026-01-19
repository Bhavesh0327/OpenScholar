use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PaperMetadata {
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
}

pub struct DiscoveryQuery {
    pub title: Option<String>,
    pub author: Option<String>,
    #[allow(dead_code)]
    pub university: Option<String>,
    pub category: Option<String>,
    pub limit: usize,
}

pub mod discovery;
pub mod resolution;
pub mod download;
pub mod legality;
