mod layers;

use crate::layers::{DiscoveryQuery};
// use crate::layers::discovery::SemanticScholarClient;
use crate::layers::resolution::Resolver;
use crate::layers::download::Downloader;
use dotenvy::dotenv;
use std::env;
use anyhow::{Result};

#[tokio::main]
async fn main() -> Result<()> {
    // 0. Load Configuration
    dotenv().ok();
    
    // Auto-create .env if it doesn't exist (copy from .env.example)
    if !std::path::Path::new(".env").exists() && std::path::Path::new(".env.example").exists() {
        println!("Creating .env from .env.example...");
        std::fs::copy(".env.example", ".env")?;
    }

    let ss_api_key = env::var("SEMANTIC_SCHOLAR_API_KEY").ok();
    let email = env::var("OPENALEX_EMAIL").ok();
    let download_dir = env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "downloads".to_string());

    // 1. Discovery (Layer 1)
    println!("--- Step 1: Discovery (Parallel) ---");
    let query = DiscoveryQuery {
        title: Some("Attention is All You Need".to_string()),
        author: None,
        university: None,
        category: Some("cs.CL".to_string()),
    };

    let orchestrator = crate::layers::discovery::DiscoveryOrchestrator::new(ss_api_key, email);
    let results = orchestrator.search_all(&query).await;
    println!("Found {} candidates from combined sources.", results.len());

    // 2. Resolution (Layer 2)
    println!("\n--- Step 2: Fuzzy Resolution ---");
    let matches = Resolver::resolve("Attention Is All You Need", results, 5);
    let sorted_matches = Resolver::sort_by_similarity(matches);
    
    if sorted_matches.is_empty() {
        println!("No close matches found.");
        return Ok(());
    }

    let (best_match, dist) = &sorted_matches[0];
    println!("Best match: {} (Levenshtein distance: {})", best_match.title, dist);

    // 3. Legality Check (Layer 3)
    if !best_match.is_oa {
        println!("The best match is not Open Access. Exiting.");
        return Ok(());
    }

    // 4. Download (Layer 4)
    println!("\n--- Step 3: Download ---");
    let downloader = Downloader::new(download_dir);
    match downloader.download_paper(best_match).await {
        Ok(path) => println!("Success! Paper and metadata downloaded to: {:?}", path),
        Err(e) => println!("Download failed: {}", e),
    }

    Ok(())
}
