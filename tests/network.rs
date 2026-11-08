//! Phase 0 baseline: HTTP behaviour of the provider clients and downloader against a local mock server.
//! `#[ignore = "GAP-.."]` tests assert desired behaviour for open gaps (docs/BASELINE.md).

mod common;
use common::{bare_paper, fixture, query, temp_dir};

use openscholar::layers::discovery::{ArxivClient, DiscoveryOrchestrator, OpenAlexClient, SemanticScholarClient};
use openscholar::layers::download::Downloader;
use openscholar::layers::PaperMetadata;
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn serve(server: &MockServer, route: &str, status: u16, body: String) {
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(server)
        .await;
}

fn local_downloader(dir: impl Into<std::path::PathBuf>) -> Downloader {
    let mut d = Downloader::new(dir);
    d.allow_private_hosts = true;
    d
}

fn oa_paper(pdf_url: String) -> PaperMetadata {
    let mut p = bare_paper();
    p.title = "Attention Is All You Need".into();
    p.authors = vec!["Ashish Vaswani".into()];
    p.year = Some(2017);
    p.doi = Some("10.48550/arXiv.1706.03762".into());
    p.is_oa = true;
    p.pdf_url = Some(pdf_url);
    p
}

// ---------- Semantic Scholar ----------

#[tokio::test]
async fn semantic_scholar_sends_key_and_parses_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/graph/v1/paper/search"))
        .and(query_param("query", "attention"))
        .and(query_param("limit", "10"))
        .and(header("x-api-key", "k123"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("semantic_scholar_search.json")))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = SemanticScholarClient::new(Some("k123".into()));
    client.base_url = server.uri();
    let papers = client.search(&query(Some("attention"), None, None, None)).await.unwrap();
    assert_eq!(papers.len(), 2);
    assert!(papers[0].abstract_text.is_some());
}

#[tokio::test]
async fn semantic_scholar_429_gives_up_after_three_attempts() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/graph/v1/paper/search"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0").set_body_string(fixture("semantic_scholar_429.json")))
        .expect(3)
        .mount(&server)
        .await;

    let mut client = SemanticScholarClient::new(None);
    client.base_url = server.uri();
    let err = client.search(&query(Some("x"), None, None, None)).await.unwrap_err();
    assert!(err.to_string().contains("429"), "{err}");
}

#[tokio::test]
async fn semantic_scholar_retries_after_429() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "0"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    serve(&server, "/graph/v1/paper/search", 200, fixture("semantic_scholar_search.json")).await;

    let mut client = SemanticScholarClient::new(None);
    client.base_url = server.uri();
    assert_eq!(client.search(&query(Some("x"), None, None, None)).await.unwrap().len(), 2);
}

#[tokio::test]
async fn semantic_scholar_malformed_body_is_an_error() {
    let server = MockServer::start().await;
    serve(&server, "/graph/v1/paper/search", 200, "<html>maintenance</html>".into()).await;

    let mut client = SemanticScholarClient::new(None);
    client.base_url = server.uri();
    assert!(client.search(&query(Some("x"), None, None, None)).await.is_err());
}

// ---------- arXiv / OpenAlex ----------

#[tokio::test]
async fn arxiv_parses_success_and_errors_on_400() {
    let server = MockServer::start().await;
    serve(&server, "/api/query", 200, fixture("arxiv_attention.xml")).await;
    let mut client = ArxivClient::new();
    client.base_url = server.uri();
    assert_eq!(client.search(&query(Some("attention"), None, None, None)).await.unwrap().len(), 2);

    let down = MockServer::start().await;
    serve(&down, "/api/query", 400, String::new()).await;
    client.base_url = down.uri();
    assert!(client.search(&query(Some("attention"), None, None, None)).await.is_err());
}

#[tokio::test]
async fn openalex_sends_mailto_and_parses_success() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/works"))
        .and(query_param("mailto", "me@example.com"))
        .and(query_param("per_page", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_string(fixture("openalex_attention.json")))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = OpenAlexClient::new(Some("me@example.com".into()));
    client.base_url = server.uri();
    let papers = client.search(&query(Some("attention"), None, None, None)).await.unwrap();
    assert_eq!(papers.len(), 1);
}

// ---------- orchestrator ----------

#[tokio::test]
async fn search_all_drops_failed_source_and_merges_duplicates() {
    let server = MockServer::start().await;
    serve(&server, "/graph/v1/paper/search", 403, String::new()).await;
    serve(&server, "/api/query", 200, fixture("arxiv_attention.xml")).await;
    serve(&server, "/works", 200, fixture("openalex_attention.json")).await;

    let mut orch = DiscoveryOrchestrator::new(None, None);
    orch.ss_client.base_url = server.uri();
    orch.arxiv_client.base_url = server.uri();
    orch.open_alex_client.base_url = server.uri();

    let (papers, statuses) = orch.search_all(&query(Some("attention"), None, None, None)).await;
    let summary: Vec<_> = statuses.iter().map(|s| (s.source.as_str(), s.ok, s.result_count, s.http_status)).collect();
    assert_eq!(summary, vec![("semantic_scholar", false, 0, Some(403)), ("arxiv", true, 2, None), ("openalex", true, 1, None)]);
    assert_eq!(papers.len(), 2);
    let merged = papers.iter().find(|p| p.id == "doi:10.48550/arxiv.1706.03762").unwrap();
    assert_eq!(merged.sources, vec!["arxiv", "openalex"]);
    assert_eq!(merged.open_alex_id.as_deref(), Some("W2626778328"));
}

// ---------- download ----------

#[tokio::test]
async fn download_writes_pdf_metadata_and_manifest() {
    let server = MockServer::start().await;
    serve(&server, "/paper.pdf", 200, "%PDF-1.4 test".into()).await;
    let dir = temp_dir("dl-ok");
    let downloader = local_downloader(&dir);
    let paper = oa_paper(format!("{}/paper.pdf", server.uri()));

    let target = downloader.download_paper(&paper).await.unwrap();
    assert_eq!(target, dir.join("10.48550_arXiv.1706.03762"));
    assert_eq!(std::fs::read_to_string(target.join("paper.pdf")).unwrap(), "%PDF-1.4 test");
    let meta: PaperMetadata = serde_json::from_str(&std::fs::read_to_string(target.join("metadata.json")).unwrap()).unwrap();
    assert_eq!(meta.title, paper.title);

    downloader.download_paper(&paper).await.unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!(manifest.as_array().unwrap().len(), 1, "re-download replaces the manifest entry");
    assert_eq!(manifest[0]["relative_path"], "10.48550_arXiv.1706.03762/paper.pdf");
}

#[tokio::test]
async fn download_http_error_creates_nothing() {
    let server = MockServer::start().await;
    serve(&server, "/missing.pdf", 404, String::new()).await;
    let dir = temp_dir("dl-404");

    let result = local_downloader(&dir).download_paper(&oa_paper(format!("{}/missing.pdf", server.uri()))).await;
    assert!(result.is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[tokio::test]
async fn download_refuses_closed_access_without_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
    let mut paper = oa_paper(format!("{}/paper.pdf", server.uri()));
    paper.is_oa = false;

    assert!(local_downloader(temp_dir("dl-closed")).download_paper(&paper).await.is_err());
}

#[tokio::test]
async fn download_rejects_html_masquerading_as_pdf() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string("<html>login</html>"))
        .mount(&server)
        .await;
    let dir = temp_dir("dl-html");

    assert!(local_downloader(&dir).download_paper(&oa_paper(format!("{}/paper.pdf", server.uri()))).await.is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[tokio::test]
async fn download_preserves_corrupt_manifest() {
    let server = MockServer::start().await;
    serve(&server, "/paper.pdf", 200, "%PDF-1.4".into()).await;
    let dir = temp_dir("dl-corrupt");
    std::fs::write(dir.join("manifest.json"), "[{\"title\": truncated").unwrap();

    let _ = local_downloader(&dir).download_paper(&oa_paper(format!("{}/paper.pdf", server.uri()))).await;
    let kept = std::fs::read_dir(&dir).unwrap().flatten().any(|e| {
        std::fs::read_to_string(e.path()).map(|c| c.contains("truncated")).unwrap_or(false)
    });
    assert!(kept, "original manifest content must survive somewhere");
}

#[tokio::test]
async fn download_refuses_local_hosts_by_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;

    let err = Downloader::new(temp_dir("dl-ssrf")).download_paper(&oa_paper(format!("{}/paper.pdf", server.uri()))).await.unwrap_err();
    assert!(err.to_string().contains("private or local host"), "{err}");
}

#[tokio::test]
async fn download_enforces_size_cap_and_leaves_no_files() {
    let server = MockServer::start().await;
    serve(&server, "/big.pdf", 200, format!("%PDF-{}", "x".repeat(2048))).await;
    let dir = temp_dir("dl-big");
    let mut downloader = local_downloader(&dir);
    downloader.max_bytes = 1024;

    assert!(downloader.download_paper(&oa_paper(format!("{}/big.pdf", server.uri()))).await.is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[tokio::test]
async fn save_unavailable_nests_by_query_and_dedupes() {
    let dir = temp_dir("unavail");
    let downloader = Downloader::new(&dir);
    let q = query(Some("attention"), Some("Vaswani"), Some("Google"), Some("cs.CL"));
    let paper = oa_paper("http://unused".into());

    downloader.save_unavailable(&q, vec![paper.clone()]).await.unwrap();
    downloader.save_unavailable(&q, vec![paper]).await.unwrap();

    let root: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("unavailable.json")).unwrap()).unwrap();
    let list = root["Google"]["cs.CL"]["Vaswani"]["attention"].as_array().unwrap();
    assert_eq!(list.len(), 1);
}
