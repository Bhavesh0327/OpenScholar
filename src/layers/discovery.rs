use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::Deserialize;
use crate::layers::{PaperMetadata, DiscoveryQuery};

#[derive(Deserialize)]
struct SSResult {
    data: Vec<SSPaper>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SSPaper {
    paper_id: String,
    title: String,
    year: Option<u32>,
    venue: Option<String>,
    #[serde(default)]
    abstract_text: Option<String>,
    #[serde(default)]
    authors: Vec<SSAuthor>,
    external_ids: Option<SSExternalIds>,
    is_open_access: Option<bool>,
    open_access_pdf: Option<SSOpenAccessPdf>,
}

#[derive(Deserialize)]
struct SSAuthor {
    name: String,
}

#[derive(Deserialize)]
struct SSExternalIds {
    #[serde(rename = "DOI")]
    doi: Option<String>,
    #[serde(rename = "ArXiv")]
    arxiv: Option<String>,
}

#[derive(Deserialize)]
struct SSOpenAccessPdf {
    url: String,
}

pub struct SemanticScholarClient {
    client: Client,
    api_key: Option<String>,
}

impl SemanticScholarClient {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            client: Client::new(),
            api_key,
        }
    }

    pub async fn search(&self, query_params: &DiscoveryQuery) -> Result<Vec<PaperMetadata>> {
        let mut query = String::new();
        if let Some(title) = &query_params.title {
            query.push_str(title);
            query.push(' ');
        }
        if let Some(author) = &query_params.author {
            query.push_str(author);
            query.push(' ');
        }
        
        let url = format!("https://api.semanticscholar.org/graph/v1/paper/search?query={}&fields=title,authors,year,venue,abstract,externalIds,isOpenAccess,openAccessPdf&limit=10", urlencoding::encode(query.trim()));
        
        let mut request = self.client.get(&url);
        if let Some(key) = &self.api_key {
            request = request.header("x-api-key", key);
        }

        let resp = request.send().await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Semantic Scholar API error: {}", resp.status()));
        }

        let results: SSResult = resp.json().await?;
        
        Ok(results.data.into_iter().map(|p| PaperMetadata {
            title: p.title,
            authors: p.authors.into_iter().map(|a| a.name).collect(),
            year: p.year,
            doi: p.external_ids.as_ref().and_then(|ids| ids.doi.clone()),
            arxiv_id: p.external_ids.as_ref().and_then(|ids| ids.arxiv.clone()),
            semantic_scholar_id: Some(p.paper_id),
            open_alex_id: None,
            venue: p.venue,
            abstract_text: p.abstract_text,
            pdf_url: p.open_access_pdf.map(|pdf| pdf.url),
            is_oa: p.is_open_access.unwrap_or(false),
            categories: Vec::new(),
        }).collect())
    }
}

pub struct ArxivClient {
    client: Client,
}

impl ArxivClient {
    pub fn new() -> Self {
        Self { client: Client::new() }
    }

    pub async fn search(&self, query_params: &DiscoveryQuery) -> Result<Vec<PaperMetadata>> {
        let mut query = String::new();
        if let Some(title) = &query_params.title {
            query.push_str(&format!("ti:\"{}\"", title));
        }
        if let Some(author) = &query_params.author {
            if !query.is_empty() { query.push_str(" AND "); }
            query.push_str(&format!("au:\"{}\"", author));
        }
        if let Some(cat) = &query_params.category {
            if !query.is_empty() { query.push_str(" AND "); }
            query.push_str(&format!("cat:\"{}\"", cat));
        }

        let url = format!("http://export.arxiv.org/api/query?search_query={}&start=0&max_results=10", urlencoding::encode(&query));
        
        let _resp = self.client.get(&url).send().await?.text().await?;
        
        // Quick and dirty XML parsing for prototype (better to use a crate like quick-xml or roxmltree)
        // For this boilerplate, we'll extract entries via simple string split or regex if needed, 
        // but let's stick to the spirit of discovery and mention we'd use a proper parser in production.
        // Returning empty for now as placeholder for full implementation.
        Ok(vec![])
    }
}

pub struct OpenAlexClient {
    client: Client,
    email: Option<String>,
}

impl OpenAlexClient {
    pub fn new(email: Option<String>) -> Self {
        Self {
            client: Client::new(),
            email,
        }
    }

    pub async fn search(&self, query_params: &DiscoveryQuery) -> Result<Vec<PaperMetadata>> {
        let mut url = format!("https://api.openalex.org/works?search={}", 
            urlencoding::encode(query_params.title.as_ref().unwrap_or(&"".to_string())));
        
        if let Some(email) = &self.email {
            url.push_str(&format!("&mailto={}", email));
        }

        let _resp = self.client.get(&url).send().await?;
        // Implementation similar to Semantic Scholar...
        Ok(vec![])
    }
}

pub struct DiscoveryOrchestrator {
    ss_client: SemanticScholarClient,
    arxiv_client: ArxivClient,
    open_alex_client: OpenAlexClient,
}

impl DiscoveryOrchestrator {
    pub fn new(ss_api_key: Option<String>, open_alex_email: Option<String>) -> Self {
        Self {
            ss_client: SemanticScholarClient::new(ss_api_key),
            arxiv_client: ArxivClient::new(),
            open_alex_client: OpenAlexClient::new(open_alex_email),
        }
    }

    pub async fn search_all(&self, query: &DiscoveryQuery) -> Vec<PaperMetadata> {
        let ss_fut = self.ss_client.search(query);
        let arxiv_fut = self.arxiv_client.search(query);
        let oa_fut = self.open_alex_client.search(query);

        let (ss_res, arxiv_res, oa_res) = tokio::join!(ss_fut, arxiv_fut, oa_fut);

        let mut all_results = Vec::new();

        match ss_res {
            Ok(results) => all_results.extend(results),
            Err(e) => println!("Warning: Semantic Scholar discovery failed: {}", e),
        }

        match arxiv_res {
            Ok(results) => all_results.extend(results),
            Err(e) => println!("Warning: arXiv discovery failed: {}", e),
        }

        match oa_res {
            Ok(results) => all_results.extend(results),
            Err(e) => println!("Warning: OpenAlex discovery failed: {}", e),
        }

        all_results
    }
}
