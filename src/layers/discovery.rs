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

use governor::{Quota, RateLimiter};
use governor::clock::DefaultClock;
use governor::state::{InMemoryState, direct::NotKeyed};
use nonzero_ext::nonzero;
use std::sync::Arc;

pub struct SemanticScholarClient {
    client: Client,
    api_key: Option<String>,
    limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
}

impl SemanticScholarClient {
    pub fn new(api_key: Option<String>) -> Self {
        // Set quota: 10 requests per second (safe default)
        // If the user asked for 500/s, but 429 happens at much lower, we start safe.
        // Let's implement what was asked: < 500. Let's go with 400.
        let quota = Quota::per_second(nonzero!(400u32));
        
        Self {
            client: Client::new(),
            api_key,
            limiter: Arc::new(RateLimiter::direct(quota)),
        }
    }

    pub async fn search(&self, query_params: &DiscoveryQuery) -> Result<Vec<PaperMetadata>> {
        // Wait for permission
        self.limiter.until_ready().await;

        let mut query = String::new();
        if let Some(title) = &query_params.title {
            query.push_str(title);
            query.push(' ');
        }
        if let Some(author) = &query_params.author {
            query.push_str(author);
            query.push(' ');
        }
        if let Some(uni) = &query_params.university {
            query.push_str(uni);
            query.push(' ');
        }
        
        let url = format!("https://api.semanticscholar.org/graph/v1/paper/search?query={}&fields=title,authors,year,venue,abstract,externalIds,isOpenAccess,openAccessPdf&limit={}", urlencoding::encode(query.trim()), query_params.limit);
        
        let mut request = self.client.get(&url);
        if let Some(key) = &self.api_key {
            request = request.header("x-api-key", key);
        }

        tracing::info!("Querying Semantic Scholar: {}", url);
        match request.send().await {
            Ok(resp) => {
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
            Err(e) => Err(anyhow!("Request failed: {}", e)),
        }
    }
}

use quick_xml::events::Event;
use quick_xml::reader::Reader;

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
        if let Some(uni) = &query_params.university {
            if !query.is_empty() { query.push_str(" AND "); }
            query.push_str(&format!("all:\"{}\"", uni));
        }

        let url = format!("http://export.arxiv.org/api/query?search_query={}&start=0&max_results={}", urlencoding::encode(&query), query_params.limit);
        tracing::info!("Querying arXiv: {}", url);
        
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return Err(anyhow!("arXiv API error: {}", resp.status()));
                }
                let text = resp.text().await?;
                
                // Manual XML Parsing with detailed extraction
                let mut reader = Reader::from_str(&text);
                reader.config_mut().trim_text(true);

                let mut papers = Vec::new();
                let mut buf = Vec::new();

                // Temp vars for current entry
                let mut in_entry = false;
                let mut title = String::new();
                let mut summary = String::new();
                let mut year = None;
                let mut authors = Vec::new();
                let mut links = Vec::new(); // (href, title, type)
                let mut id = String::new();

                // Parsing State
                #[derive(PartialEq)]
                enum TagState { None, Title, Summary, Published, AuthorName, Id }
                let mut state = TagState::None;

                loop {
                    match reader.read_event_into(&mut buf) {
                        Ok(Event::Start(e)) => {
                            match e.name().as_ref() {
                                b"entry" => {
                                    in_entry = true;
                                    title.clear(); summary.clear(); year = None; authors.clear(); links.clear(); id.clear();
                                },
                                b"title" if in_entry => state = TagState::Title,
                                b"summary" if in_entry => state = TagState::Summary,
                                b"published" if in_entry => state = TagState::Published,
                                b"name" if in_entry => state = TagState::AuthorName,
                                b"id" if in_entry => state = TagState::Id,
                                // 'link' with content (rare for Atom)
                                b"link" if in_entry => {
                                    let mut href = String::new();
                                    let mut title_attr = String::new();
                                    let mut type_attr = String::new();
                                    for attr in e.attributes().flatten() {
                                        match attr.key.as_ref() {
                                            b"href" => href = String::from_utf8_lossy(&attr.value).to_string(),
                                            b"title" => title_attr = String::from_utf8_lossy(&attr.value).to_string(),
                                            b"type" => type_attr = String::from_utf8_lossy(&attr.value).to_string(),
                                            _ => {}
                                        }
                                    }
                                    links.push((href, title_attr, type_attr));
                                }
                                _ => state = TagState::None,
                            }
                        }
                        Ok(Event::Empty(e)) => {
                            match e.name().as_ref() {
                                b"link" if in_entry => {
                                    let mut href = String::new();
                                    let mut title_attr = String::new();
                                    let mut type_attr = String::new();
                                    for attr in e.attributes().flatten() {
                                        match attr.key.as_ref() {
                                            b"href" => href = String::from_utf8_lossy(&attr.value).to_string(),
                                            b"title" => title_attr = String::from_utf8_lossy(&attr.value).to_string(),
                                            b"type" => type_attr = String::from_utf8_lossy(&attr.value).to_string(),
                                            _ => {}
                                        }
                                    }
                                    tracing::debug!("Found (empty) link in entry {}: href={}, title={}, type={}", id.clone(), href, title_attr, type_attr);
                                    links.push((href, title_attr, type_attr));
                                }
                                _ => {}
                            }
                        }
                        Ok(Event::Text(e)) => {
                            if in_entry {
                                let txt = String::from_utf8_lossy(&e.into_inner()).into_owned();
                                match state {
                                    TagState::Title => title = txt,
                                    TagState::Summary => summary = txt,
                                    TagState::Published => {
                                        if let Some(y_str) = txt.split('-').next() {
                                            year = y_str.parse().ok();
                                        }
                                    }
                                    TagState::AuthorName => authors.push(txt),
                                    TagState::Id => id = txt,
                                    _ => {}
                                }
                            }
                        }
                        Ok(Event::End(e)) => {
                            match e.name().as_ref() {
                                b"entry" => {
                                    in_entry = false;
                                    tracing::info!("Parsed Arxiv Entry: Title='{}', ID='{}', Links={}", title, id, links.len());
                                    
                                    let pdf_url = links.iter()
                                        .find(|(_, t, ty)| t == "pdf" || ty == "application/pdf")
                                        .map(|(h, _, _)| h.clone());

                                    papers.push(PaperMetadata {
                                        title: title.replace('\n', " ").trim().to_string(),
                                        authors: authors.clone(),
                                        year,
                                        doi: None,
                                        arxiv_id: Some(id.clone()),
                                        semantic_scholar_id: None,
                                        open_alex_id: None,
                                        venue: Some("arXiv".to_string()),
                                        abstract_text: Some(summary.trim().to_string()),
                                        pdf_url: pdf_url.clone().map(|u| {
                                             tracing::info!("Found arXiv PDF link: {}", u);
                                             u
                                        }),
                                        is_oa: true,
                                        categories: Vec::new(),
                                    });
                                },
                                _ => state = TagState::None,
                            }
                        }
                        Ok(Event::Eof) => break,
                        Err(e) => {
                            tracing::warn!("XML parsing error at position {}: {:?}", reader.buffer_position(), e);
                            break;
                        }
                        _ => {}
                    }
                    buf.clear();
                }

                Ok(papers)
            }
            Err(e) => Err(anyhow!("Request failed: {}", e)),
        }
    }
}

// OpenAlex Data Structures
#[derive(Deserialize)]
struct OAResponse {
    results: Vec<OAWork>,
}

#[derive(Deserialize)]
struct OAWork {
    id: String,
    title: Option<String>,
    publication_year: Option<u32>,
    ids: Option<OAIds>,
    authorships: Vec<OAAuthorship>,
    best_oa_location: Option<OALocation>,
    #[serde(default)]
    #[allow(dead_code)]
    abstract_inverted_index: Option<serde_json::Value>, // We won't reconstruct abstract for now, complex
}

#[derive(Deserialize)]
struct OAIds {
    doi: Option<String>,
    #[serde(rename = "openalex")]
    #[allow(dead_code)]
    openalex: Option<String>,
}

#[derive(Deserialize)]
struct OAAuthorship {
    author: OAAuthor,
}

#[derive(Deserialize)]
struct OAAuthor {
    display_name: String,
}

#[derive(Deserialize)]
struct OALocation {
    pdf_url: Option<String>,
    is_oa: bool,
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
        // Use 'filter' for institution if provided, otherwise 'search'
        let mut url = "https://api.openalex.org/works?".to_string();
        
        let mut filters = Vec::new();
        if let Some(uni) = &query_params.university {
            // Encode value but keep key and colon raw if possible, or handle carefully.
            // OpenAlex expects filter=key:value. 
            // We shouldn't encode the colon if possible, but we must encode the value.
            filters.push(format!("institutions.display_name:{}", uni));
        }

        let mut search_parts = Vec::new();
        if let Some(title) = &query_params.title {
             search_parts.push(title.clone());
        }
        if let Some(author) = &query_params.author {
             search_parts.push(author.clone());
        }

        if !filters.is_empty() {
             url.push_str("filter=");
             // Manually build the filter string: key:value,key2:value2
             // We need to encode the VALUES.
             let encoded_filters: Vec<String> = filters.iter().map(|f| {
                 let parts: Vec<&str> = f.splitn(2, ':').collect();
                 if parts.len() == 2 {
                     format!("{}:{}", parts[0], urlencoding::encode(parts[1]))
                 } else {
                     f.clone() // fallback
                 }
             }).collect();
             url.push_str(&encoded_filters.join(","));
             
             if !search_parts.is_empty() {
                 url.push_str("&");
             }
        }

        if !search_parts.is_empty() {
            url.push_str("search=");
            url.push_str(&urlencoding::encode(&search_parts.join(" ")));
        }

        // Add limit
        if url.contains('?') {
            url.push_str("&");
        } else {
            url.push_str("?");
        }
        url.push_str(&format!("per_page={}", query_params.limit));
        
        if let Some(email) = &self.email {
            url.push_str(&format!("&mailto={}", email));
        }

        tracing::info!("Querying OpenAlex: {}", url);
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return Err(anyhow!("OpenAlex API error: {}", resp.status()));
                }
                let oa_resp: OAResponse = resp.json().await?;
                
                Ok(oa_resp.results.into_iter().map(|work| {
                    let authors = work.authorships.into_iter().map(|a| a.author.display_name).collect();
                    PaperMetadata {
                        title: work.title.unwrap_or_else(|| "Untitled".to_string()),
                        authors,
                        year: work.publication_year,
                        doi: work.ids.as_ref().and_then(|ids| ids.doi.clone()),
                        // OpenAlex doesn't always give Arxiv ID easily in top level IDs, 
                        // sometimes it's in detailed location. Skipping for now.
                        arxiv_id: None, 
                        semantic_scholar_id: None,
                        open_alex_id: Some(work.id),
                        venue: None, // Could parse, but skipping for brevity
                        abstract_text: None, // Requires reconstructing from inverted index
                        pdf_url: work.best_oa_location.as_ref().and_then(|loc| loc.pdf_url.clone()),
                        is_oa: work.best_oa_location.map(|loc| loc.is_oa).unwrap_or(false),
                        categories: Vec::new(),
                    }
                }).collect())
            }
            Err(e) => Err(anyhow!("Request failed: {}", e)),
        }
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
            Err(e) => tracing::warn!("Semantic Scholar discovery failed: {}", e),
        }

        match arxiv_res {
            Ok(results) => all_results.extend(results),
            Err(e) => tracing::warn!("arXiv discovery failed: {}", e),
        }

        match oa_res {
            Ok(results) => all_results.extend(results),
            Err(e) => tracing::warn!("OpenAlex discovery failed: {}", e),
        }

        all_results
    }
}
