#![allow(dead_code)]

use openscholar::layers::{DiscoveryQuery, PaperMetadata};

pub fn fixture(name: &str) -> String {
    let path = format!("{}/tests/fixtures/{}", env!("CARGO_MANIFEST_DIR"), name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("missing fixture {path}: {e}"))
}

pub fn query(title: Option<&str>, author: Option<&str>, university: Option<&str>, category: Option<&str>) -> DiscoveryQuery {
    DiscoveryQuery {
        title: title.map(String::from),
        author: author.map(String::from),
        university: university.map(String::from),
        category: category.map(String::from),
        topic: None,
        limit: 10,
    }
}

pub fn bare_paper() -> PaperMetadata {
    PaperMetadata {
        id: String::new(),
        title: "T".into(),
        authors: vec![],
        year: None,
        doi: None,
        arxiv_id: None,
        semantic_scholar_id: None,
        open_alex_id: None,
        venue: None,
        abstract_text: None,
        pdf_url: None,
        is_oa: false,
        categories: vec![],
        sources: vec![],
    }
}

pub fn temp_dir(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("openscholar-{}-{}-{}", tag, std::process::id(), nanos));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
