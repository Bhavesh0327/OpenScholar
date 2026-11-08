//! Phase 0 baseline: provider parsing and URL construction against recorded fixtures.
//!
//! Passing tests pin current behaviour. Tests marked `#[ignore = "GAP-.."]` assert the
//! *desired* behaviour for a known gap (see docs/BASELINE.md); run them with
//! `cargo test -- --ignored` and un-ignore each one as its fix lands.

use openscholar::layers::discovery::{ArxivClient, OpenAlexClient, SemanticScholarClient};
use openscholar::layers::download::Downloader;

mod common;
use common::{bare_paper, fixture, query};

// ---------- arXiv ----------

#[test]
fn arxiv_parses_recorded_feed() {
    let papers = ArxivClient::parse_feed(&fixture("arxiv_attention.xml"));
    assert_eq!(papers.len(), 2, "feed-level <title>/<id> must not become entries");

    let p = &papers[0];
    assert_eq!(p.title, "Attention Is All You Need");
    assert_eq!(p.year, Some(2017));
    assert_eq!(p.pdf_url.as_deref(), Some("https://arxiv.org/pdf/1706.03762v7"));
    assert!(p.is_oa);
    assert_eq!(p.venue.as_deref(), Some("arXiv"));
    assert!(p.authors.iter().any(|a| a == "Ashish Vaswani"));
    assert!(p.abstract_text.as_deref().unwrap_or("").len() > 100);
}

#[test]
fn arxiv_id_is_currently_the_full_abs_url() {
    // Baseline: arxiv_id holds the Atom <id> verbatim (URL + version), not "1706.03762".
    let papers = ArxivClient::parse_feed(&fixture("arxiv_attention.xml"));
    assert_eq!(papers[0].arxiv_id.as_deref(), Some("http://arxiv.org/abs/1706.03762v7"));
}

#[test]
fn arxiv_malformed_feed_returns_partial_results_without_error() {
    let papers = ArxivClient::parse_feed("<feed><entry><title>Only</title></entry><entry><title>Bro");
    assert_eq!(papers.len(), 1);
}

#[test]
fn arxiv_titles_with_entities_are_decoded() {
    let papers = ArxivClient::parse_feed(&fixture("arxiv_entities.xml"));
    assert_eq!(papers[0].title, "Recent progress of the ATLAS Planar Pixel Sensor R&D Project");
    assert_eq!(papers[2].title, "Fast timing silicon R$\\&$D for the future Electron-Ion Collider");
}

#[test]
fn arxiv_entities_keep_surrounding_spaces_and_numeric_refs() {
    let feed = "<feed><entry><id> http://arxiv.org/abs/1v1 </id><title>Q &amp; A &#38; &lt;more&gt; &unknown;</title>\
        <author><name>Ann &#x00E9;l</name></author><author><name>Bob</name></author></entry></feed>";
    let p = &ArxivClient::parse_feed(feed)[0];
    assert_eq!(p.title, "Q & A & <more> &unknown;");
    assert_eq!(p.authors, vec!["Ann \u{e9}l", "Bob"]);
    assert_eq!(p.arxiv_id.as_deref(), Some("http://arxiv.org/abs/1v1"));
}

#[test]
#[ignore = "GAP-04: arXiv IDs are not canonicalized"]
fn arxiv_id_is_canonical() {
    let papers = ArxivClient::parse_feed(&fixture("arxiv_attention.xml"));
    assert_eq!(papers[0].arxiv_id.as_deref(), Some("1706.03762"));
}

#[test]
fn arxiv_url_quotes_title_as_exact_phrase() {
    let url = ArxivClient::build_url("http://export.arxiv.org", &query(Some("deep learning"), Some("Hinton"), None, None));
    assert!(url.starts_with("http://export.arxiv.org/api/query?"));
    assert!(url.contains(&*urlencoding::encode("ti:\"deep learning\" AND au:\"Hinton\"")));
}

// ---------- Semantic Scholar (synthetic fixture, see _fixture_note) ----------

#[test]
fn semantic_scholar_parses_search_response() {
    let papers = SemanticScholarClient::parse_response(&fixture("semantic_scholar_search.json")).unwrap();
    assert_eq!(papers.len(), 2);

    let p = &papers[0];
    assert_eq!(p.title, "Attention is All you Need");
    assert_eq!(p.doi.as_deref(), Some("10.48550/arXiv.1706.03762"));
    assert_eq!(p.arxiv_id.as_deref(), Some("1706.03762"));
    assert_eq!(p.semantic_scholar_id.as_deref(), Some("204e3073870fae3d05bcbc2f6a8e263d9b72e776"));
    assert_eq!(p.pdf_url.as_deref(), Some("https://arxiv.org/pdf/1706.03762"));
    assert!(p.is_oa);

    let closed = &papers[1];
    assert!(!closed.is_oa);
    assert!(closed.pdf_url.is_none() && closed.doi.is_none());
}

#[test]
fn semantic_scholar_rate_limit_body_is_a_parse_error() {
    // Real anonymous 429 body. The client returns Err on non-2xx before parsing; this pins
    // that the body alone would not silently parse as "zero results".
    assert!(SemanticScholarClient::parse_response(&fixture("semantic_scholar_429.json")).is_err());
}

#[test]
fn semantic_scholar_abstract_is_kept() {
    let papers = SemanticScholarClient::parse_response(&fixture("semantic_scholar_search.json")).unwrap();
    assert_eq!(
        papers[0].abstract_text.as_deref(),
        Some("The dominant sequence transduction models are based on complex recurrent or convolutional neural networks.")
    );
    assert!(papers[1].abstract_text.is_none(), "null abstract stays None");
}

#[test]
fn semantic_scholar_url_joins_all_fields_into_one_query() {
    let url = SemanticScholarClient::build_url("https://api.semanticscholar.org", &query(Some("graph nets"), None, Some("MIT"), Some("cs.LG")));
    assert!(url.contains("query=graph%20nets%20MIT%20cs.LG&"));
    assert!(url.ends_with("&limit=10"));
}

// ---------- OpenAlex ----------

#[test]
fn openalex_parses_recorded_works() {
    let papers = OpenAlexClient::parse_response(&fixture("openalex_attention.json")).unwrap();
    assert_eq!(papers.len(), 1);

    let p = &papers[0];
    assert_eq!(p.title, "Attention Is All You Need");
    assert_eq!(p.year, Some(2017));
    assert_eq!(p.open_alex_id.as_deref(), Some("https://openalex.org/W2626778328"));
    assert_eq!(p.doi.as_deref(), Some("https://doi.org/10.48550/arxiv.1706.03762"));
    assert_eq!(p.pdf_url.as_deref(), Some("https://arxiv.org/pdf/1706.03762"));
    assert!(p.is_oa);
    assert_eq!(p.authors.len(), 8);
}

#[test]
fn openalex_abstract_is_reconstructed() {
    let papers = OpenAlexClient::parse_response(&fixture("openalex_attention.json")).unwrap();
    assert!(papers[0].abstract_text.as_deref().unwrap_or("").starts_with("The dominant sequence transduction models"));
}

#[test]
fn openalex_abstract_handles_repeated_words_and_null_index() {
    let body = r#"{"results":[
        {"id":"W1","title":"a","authorships":[],"abstract_inverted_index":{"to":[0,2],"be":[1,3],"or":[4]}},
        {"id":"W2","title":"b","authorships":[],"abstract_inverted_index":null}]}"#;
    let papers = OpenAlexClient::parse_response(body).unwrap();
    assert_eq!(papers[0].abstract_text.as_deref(), Some("to be to be or"));
    assert!(papers[1].abstract_text.is_none());
}

#[test]
fn openalex_url_uses_affiliation_filter_and_search() {
    let url = OpenAlexClient::build_url("https://api.openalex.org", &query(Some("rust"), None, Some("ETH Zurich"), None), Some("me@example.com"));
    assert_eq!(
        url,
        "https://api.openalex.org/works?filter=raw_affiliation_strings.search:ETH%20Zurich&search=rust&per_page=10&mailto=me@example.com"
    );
}

// ---------- cross-provider identity ----------

#[test]
#[ignore = "GAP-04: DOIs are not normalized across providers, so the same paper cannot be deduplicated"]
fn same_paper_has_same_doi_across_providers() {
    let ss = SemanticScholarClient::parse_response(&fixture("semantic_scholar_search.json")).unwrap();
    let oa = OpenAlexClient::parse_response(&fixture("openalex_attention.json")).unwrap();
    assert_eq!(ss[0].doi, oa[0].doi);
}

#[test]
fn storage_id_baseline_differs_for_same_paper_across_providers() {
    // Baseline: the same paper from SS and OpenAlex lands in two different download dirs.
    let ss = SemanticScholarClient::parse_response(&fixture("semantic_scholar_search.json")).unwrap();
    let oa = OpenAlexClient::parse_response(&fixture("openalex_attention.json")).unwrap();
    assert_eq!(Downloader::storage_id(&ss[0]), "10.48550_arXiv.1706.03762");
    assert_eq!(Downloader::storage_id(&oa[0]), "doi.org_10.48550_arxiv.1706.03762");
}

#[test]
#[ignore = "GAP-05: papers with no DOI/arXiv/S2 ID all share the `unknown_id` directory"]
fn storage_id_is_unique_for_openalex_only_papers() {
    let mut a = bare_paper();
    a.open_alex_id = Some("https://openalex.org/W1".into());
    let mut b = bare_paper();
    b.open_alex_id = Some("https://openalex.org/W2".into());
    assert_ne!(Downloader::storage_id(&a), Downloader::storage_id(&b));
}
