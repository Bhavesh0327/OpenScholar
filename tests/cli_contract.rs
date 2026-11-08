//! Contract for unattended use: `--json` stdout schema, stderr diagnostics and exit codes.

mod common;
use common::{fixture, temp_dir};

use openscholar::layers::LiteratureResult;
use tokio::process::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn missing_search_args_exit_2_with_error_on_stderr() {
    let out = Command::new(env!("CARGO_BIN_EXE_openscholar"))
        .current_dir(temp_dir("cli-noargs"))
        .output()
        .await
        .unwrap();

    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("error:"));
}

#[tokio::test]
async fn json_metadata_only_reports_papers_and_source_statuses() {
    let server = MockServer::start().await;
    for (route, status, body) in [
        ("/graph/v1/paper/search", 403, String::new()),
        ("/api/query", 200, fixture("arxiv_attention.xml")),
        ("/works", 200, fixture("openalex_attention.json")),
    ] {
        Mock::given(method("GET")).and(path(route))
            .respond_with(ResponseTemplate::new(status).set_body_string(body))
            .mount(&server).await;
    }
    let cwd = temp_dir("cli-json");

    let out = Command::new(env!("CARGO_BIN_EXE_openscholar"))
        .args(["--topic", "attention", "--json", "--metadata-only"])
        .current_dir(&cwd)
        .env("OPENSCHOLAR_SEMANTIC_SCHOLAR_URL", server.uri())
        .env("OPENSCHOLAR_ARXIV_URL", server.uri())
        .env("OPENSCHOLAR_OPENALEX_URL", server.uri())
        .env("DOWNLOAD_DIR", cwd.join("downloads"))
        .env_remove("SEMANTIC_SCHOLAR_API_KEY")
        .output()
        .await
        .unwrap();

    assert_eq!(out.status.code(), Some(0));
    let result: LiteratureResult = serde_json::from_slice(&out.stdout).expect("stdout is only the JSON result");
    assert_eq!(result.schema_version, 1);
    assert_eq!(result.papers.len(), 2);
    assert!(result.downloads.is_empty());
    let ok: Vec<_> = result.source_statuses.iter().map(|s| (s.source.as_str(), s.ok)).collect();
    assert_eq!(ok, vec![("semantic_scholar", false), ("arxiv", true), ("openalex", true)]);
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning: semantic_scholar failed"));
    assert!(!cwd.join("downloads").exists(), "--metadata-only writes nothing");
}
