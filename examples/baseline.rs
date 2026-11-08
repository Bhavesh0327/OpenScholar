//! Live baseline against the real providers: `cargo run --example baseline > baseline.json`.
//! Not run in CI; numbers depend on network, rate limits and API keys.

use openscholar::layers::discovery::{ArxivClient, OpenAlexClient, SemanticScholarClient};
use openscholar::layers::{DiscoveryQuery, PaperMetadata};
use std::collections::HashSet;
use std::time::Instant;

fn q(title: Option<&str>, author: Option<&str>, university: Option<&str>) -> DiscoveryQuery {
    DiscoveryQuery {
        title: title.map(String::from),
        author: author.map(String::from),
        university: university.map(String::from),
        category: None,
        limit: 10,
    }
}

fn record(source: &str, label: &str, started: Instant, result: anyhow::Result<Vec<PaperMetadata>>) -> serde_json::Value {
    let latency_ms = started.elapsed().as_millis();
    match result {
        Ok(papers) => serde_json::json!({
            "source": source, "query": label, "ok": true, "latency_ms": latency_ms,
            "count": papers.len(),
            "with_pdf": papers.iter().filter(|p| p.is_oa && p.pdf_url.is_some()).count(),
            "with_abstract": papers.iter().filter(|p| p.abstract_text.is_some()).count(),
            "titles": papers.iter().map(|p| p.title.to_lowercase()).collect::<Vec<_>>(),
        }),
        Err(e) => serde_json::json!({"source": source, "query": label, "ok": false, "latency_ms": latency_ms, "error": e.to_string()}),
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    let ss = SemanticScholarClient::new(std::env::var("SEMANTIC_SCHOLAR_API_KEY").ok().filter(|k| !k.is_empty()));
    let arxiv = ArxivClient::new();
    let oa = OpenAlexClient::new(std::env::var("OPENALEX_EMAIL").ok());

    let queries = [
        ("exact_title", q(Some("Attention Is All You Need"), None, None)),
        ("topic", q(Some("retrieval augmented generation evaluation"), None, None)),
        ("author", q(None, Some("Yoshua Bengio"), None)),
        ("university", q(Some("graph neural networks"), None, Some("Stanford University"))),
    ];

    let mut runs = Vec::new();
    for (label, query) in &queries {
        let t = Instant::now();
        runs.push(record("semantic_scholar", label, t, ss.search(query).await));
        let t = Instant::now();
        runs.push(record("arxiv", label, t, arxiv.search(query).await));
        let t = Instant::now();
        runs.push(record("openalex", label, t, oa.search(query).await));
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;

        let titles: Vec<&str> = runs.iter().rev().take(3)
            .filter_map(|r| r["titles"].as_array())
            .flatten()
            .filter_map(|t| t.as_str())
            .collect();
        let unique: HashSet<&str> = titles.iter().copied().collect();
        runs.push(serde_json::json!({"source": "combined", "query": label, "count": titles.len(), "duplicate_titles": titles.len() - unique.len()}));
    }

    println!("{}", serde_json::to_string_pretty(&serde_json::json!({
        "recorded_at": chrono::Utc::now().to_rfc3339(),
        "runs": runs,
    })).unwrap());
}
