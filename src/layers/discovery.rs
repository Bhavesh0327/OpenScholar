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
    #[serde(default, rename = "abstract")]
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
    pub base_url: String,
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
            base_url: "https://api.semanticscholar.org".to_string(),
            client: Client::new(),
            api_key,
            limiter: Arc::new(RateLimiter::direct(quota)),
        }
    }

    pub async fn search(&self, query_params: &DiscoveryQuery) -> Result<Vec<PaperMetadata>> {
        // Wait for permission
        self.limiter.until_ready().await;

        let url = Self::build_url(&self.base_url, query_params);

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
                Self::parse_response(&resp.text().await?)
            }
            Err(e) => Err(anyhow!("Request failed: {}", e)),
        }
    }

    pub fn build_url(base_url: &str, query_params: &DiscoveryQuery) -> String {
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
        if let Some(cat) = &query_params.category {
            query.push_str(cat);
            query.push(' ');
        }
        
        format!("{}/graph/v1/paper/search?query={}&fields=title,authors,year,venue,abstract,externalIds,isOpenAccess,openAccessPdf&limit={}", base_url, urlencoding::encode(query.trim()), query_params.limit)
    }

    /// Parse a successful `/paper/search` response body. Pure, so it can be fixture-tested.
    pub fn parse_response(body: &str) -> Result<Vec<PaperMetadata>> {
        let results: SSResult = serde_json::from_str(body)?;
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

use quick_xml::events::Event;
use quick_xml::reader::Reader;

pub struct ArxivClient {
    pub base_url: String,
    client: Client,
}

impl ArxivClient {
    pub fn new() -> Self {
        Self { base_url: "http://export.arxiv.org".to_string(), client: Client::new() }
    }

    pub async fn search(&self, query_params: &DiscoveryQuery) -> Result<Vec<PaperMetadata>> {
        let url = Self::build_url(&self.base_url, query_params);
        tracing::info!("Querying arXiv: {}", url);

        match self.client.get(&url).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return Err(anyhow!("arXiv API error: {}", resp.status()));
                }
                let text = resp.text().await?;
                Ok(Self::parse_feed(&text))
            }
            Err(e) => Err(anyhow!("Request failed: {}", e)),
        }
    }

    pub fn build_url(base_url: &str, query_params: &DiscoveryQuery) -> String {
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
            // 'cat' usually requires strict taxonomy (e.g. cs.CR). 
            // Users tend to type keywords ("Cybersecurity"), so 'all' is safer.
            query.push_str(&format!("all:\"{}\"", cat));
        }
        if let Some(uni) = &query_params.university {
            if !query.is_empty() { query.push_str(" AND "); }
            query.push_str(&format!("all:\"{}\"", uni));
        }

        format!("{}/api/query?search_query={}&start=0&max_results={}", base_url, urlencoding::encode(&query), query_params.limit)
    }

    /// Parse an arXiv Atom feed. Pure, so it can be fixture-tested.
    pub fn parse_feed(text: &str) -> Vec<PaperMetadata> {
        // Manual XML Parsing with detailed extraction
        // No trim_text: quick-xml splits text at entity refs, and trimming each piece would drop spaces around them.
        let mut reader = Reader::from_str(&text);

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
                        b"name" if in_entry => {
                            state = TagState::AuthorName;
                            authors.push(String::new());
                        }
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
                Ok(ev @ (Event::Text(_) | Event::GeneralRef(_))) => {
                    if in_entry {
                        let txt = match ev {
                            Event::Text(e) => String::from_utf8_lossy(&e.into_inner()).into_owned(),
                            Event::GeneralRef(e) => match e.resolve_char_ref() {
                                Ok(Some(c)) => c.to_string(),
                                _ => {
                                    let name = String::from_utf8_lossy(&e).into_owned();
                                    quick_xml::escape::resolve_predefined_entity(&name)
                                        .map(String::from)
                                        .unwrap_or_else(|| format!("&{};", name))
                                }
                            },
                            _ => unreachable!(),
                        };
                        match state {
                            TagState::Title => title.push_str(&txt),
                            TagState::Summary => summary.push_str(&txt),
                            TagState::Published => {
                                if let Some(y_str) = txt.trim().split('-').next() {
                                    year = y_str.parse().ok();
                                }
                            }
                            TagState::AuthorName => if let Some(a) = authors.last_mut() { a.push_str(&txt) },
                            TagState::Id => id.push_str(&txt),
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
                                authors: authors.iter().map(|a| a.trim().to_string()).collect(),
                                year,
                                doi: None,
                                arxiv_id: Some(id.trim().to_string()),
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

        papers
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
    abstract_inverted_index: Option<std::collections::HashMap<String, Vec<usize>>>,
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
    pub base_url: String,
    client: Client,
    email: Option<String>,
}

impl OpenAlexClient {
    pub fn new(email: Option<String>) -> Self {
        Self {
            base_url: "https://api.openalex.org".to_string(),
            client: Client::new(),
            email,
        }
    }

    pub async fn search(&self, query_params: &DiscoveryQuery) -> Result<Vec<PaperMetadata>> {
        let url = Self::build_url(&self.base_url, query_params, self.email.as_deref());

        tracing::info!("Querying OpenAlex: {}", url);
        match self.client.get(&url).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return Err(anyhow!("OpenAlex API error: {}", resp.status()));
                }
                Self::parse_response(&resp.text().await?)
            }
            Err(e) => Err(anyhow!("Request failed: {}", e)),
        }
    }

    pub fn build_url(base_url: &str, query_params: &DiscoveryQuery, email: Option<&str>) -> String {
        // Use 'filter' for institution if provided, otherwise 'search'
        let mut url = format!("{}/works?", base_url);
        
        let mut filters = Vec::new();
        if let Some(uni) = &query_params.university {
            // Encode value but keep key and colon raw if possible, or handle carefully.
            // OpenAlex expects filter=key:value. 
            // We shouldn't encode the colon if possible, but we must encode the value.
            filters.push(format!("raw_affiliation_strings.search:{}", uni));
        }

        let mut search_parts = Vec::new();
        if let Some(title) = &query_params.title {
             search_parts.push(title.clone());
        }
        if let Some(author) = &query_params.author {
             search_parts.push(author.clone());
        }
        if let Some(cat) = &query_params.category {
             search_parts.push(cat.clone());
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
        
        if let Some(email) = email {
            url.push_str(&format!("&mailto={}", email));
        }

        url
    }

    /// Parse a successful `/works` response body. Pure, so it can be fixture-tested.
    pub fn parse_response(body: &str) -> Result<Vec<PaperMetadata>> {
        let oa_resp: OAResponse = serde_json::from_str(body)?;

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
                abstract_text: work.abstract_inverted_index.map(|index| {
                    let mut words: Vec<(usize, String)> = index.into_iter()
                        .flat_map(|(word, positions)| positions.into_iter().map(move |pos| (pos, word.clone())))
                        .collect();
                    words.sort_by_key(|(pos, _)| *pos);
                    words.into_iter().map(|(_, w)| w).collect::<Vec<_>>().join(" ")
                }),
                pdf_url: work.best_oa_location.as_ref().and_then(|loc| loc.pdf_url.clone()),
                is_oa: work.best_oa_location.map(|loc| loc.is_oa).unwrap_or(false),
                categories: Vec::new(),
            }
        }).collect())
    }
}

pub struct DiscoveryOrchestrator {
    pub ss_client: SemanticScholarClient,
    pub arxiv_client: ArxivClient,
    pub open_alex_client: OpenAlexClient,
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
