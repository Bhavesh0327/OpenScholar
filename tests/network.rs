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
async fn semantic_scholar_429_is_an_error_and_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/graph/v1/paper/search"))
        .respond_with(ResponseTemplate::new(429).set_body_string(fixture("semantic_scholar_429.json")))
        .expect(1)
        .mount(&server)
        .await;

    let mut client = SemanticScholarClient::new(None);
    client.base_url = server.uri();
    let err = client.search(&query(Some("x"), None, None, None)).await.unwrap_err();
    assert!(err.to_string().contains("429"), "{err}");
}

#[tokio::test]
#[ignore = "GAP-10: no retry/backoff on 429"]
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
async fn arxiv_parses_success_and_errors_on_503() {
    let server = MockServer::start().await;
    serve(&server, "/api/query", 200, fixture("arxiv_attention.xml")).await;
    let mut client = ArxivClient::new();
    client.base_url = server.uri();
    assert_eq!(client.search(&query(Some("attention"), None, None, None)).await.unwrap().len(), 2);

    let down = MockServer::start().await;
    serve(&down, "/api/query", 503, String::new()).await;
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
async fn search_all_drops_failed_source_and_keeps_duplicates() {
    let server = MockServer::start().await;
    serve(&server, "/graph/v1/paper/search", 429, fixture("semantic_scholar_429.json")).await;
    serve(&server, "/api/query", 200, fixture("arxiv_attention.xml")).await;
    serve(&server, "/works", 200, fixture("openalex_attention.json")).await;

    let mut orch = DiscoveryOrchestrator::new(None, None);
    orch.ss_client.base_url = server.uri();
    orch.arxiv_client.base_url = server.uri();
    orch.open_alex_client.base_url = server.uri();

    let papers = orch.search_all(&query(Some("attention"), None, None, None)).await;
    // Baseline: SS failure is silent (GAP-11) and the same paper from arXiv + OpenAlex is not merged (GAP-04).
    assert_eq!(papers.len(), 3);
    assert_eq!(papers.iter().filter(|p| p.title == "Attention Is All You Need").count(), 2);
}

// ---------- download ----------

#[tokio::test]
async fn download_writes_pdf_metadata_and_manifest() {
    let server = MockServer::start().await;
    serve(&server, "/paper.pdf", 200, "%PDF-1.4 test".into()).await;
    let dir = temp_dir("dl-ok");
    let downloader = Downloader::new(&dir);
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

    let result = Downloader::new(&dir).download_paper(&oa_paper(format!("{}/missing.pdf", server.uri()))).await;
    assert!(result.is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[tokio::test]
async fn download_refuses_closed_access_without_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200)).expect(0).mount(&server).await;
    let mut paper = oa_paper(format!("{}/paper.pdf", server.uri()));
    paper.is_oa = false;

    assert!(Downloader::new(temp_dir("dl-closed")).download_paper(&paper).await.is_err());
}

#[tokio::test]
#[ignore = "GAP-12: non-PDF responses are saved as paper.pdf"]
async fn download_rejects_html_masquerading_as_pdf() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type", "text/html").set_body_string("<html>login</html>"))
        .mount(&server)
        .await;
    let dir = temp_dir("dl-html");

    assert!(Downloader::new(&dir).download_paper(&oa_paper(format!("{}/paper.pdf", server.uri()))).await.is_err());
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
}

#[tokio::test]
#[ignore = "GAP-13: a corrupt manifest.json is silently replaced, losing its entries"]
async fn download_preserves_corrupt_manifest() {
    let server = MockServer::start().await;
    serve(&server, "/paper.pdf", 200, "%PDF-1.4".into()).await;
    let dir = temp_dir("dl-corrupt");
    std::fs::write(dir.join("manifest.json"), "[{\"title\": truncated").unwrap();

    let _ = Downloader::new(&dir).download_paper(&oa_paper(format!("{}/paper.pdf", server.uri()))).await;
    let kept = std::fs::read_dir(&dir).unwrap().flatten().any(|e| {
        std::fs::read_to_string(e.path()).map(|c| c.contains("truncated")).unwrap_or(false)
    });
    assert!(kept, "original manifest content must survive somewhere");
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
