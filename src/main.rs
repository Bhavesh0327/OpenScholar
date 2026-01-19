mod layers;

use crate::layers::{DiscoveryQuery};
use crate::layers::resolution::Resolver;
use crate::layers::download::Downloader;
use crate::layers::discovery::DiscoveryOrchestrator;
use dotenvy::dotenv;
use std::env;
use anyhow::{Result};
use clap::Parser;

/// OpenScholar: Intelligent Research Discovery Engine
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Title of the paper
    #[arg(short, long)]
    title: Option<String>,

    /// Author of the paper
    #[arg(short, long)]
    author: Option<String>,

    /// Category of the paper (e.g., cs.ML, quant-ph)
    #[arg(short, long)]
    category: Option<String>,

    /// University affiliation
    #[arg(short, long)]
    university: Option<String>,

    /// Custom Levenshtein threshold for fuzzy matching
    #[arg(long, default_value_t = 5)]
    threshold: usize,

    /// Maximum number of results to return
    #[arg(short = 'n', long, default_value_t = 10)]
    limit: usize,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 0. Load Configuration
    dotenv().ok();
    
    // Initialize Logging
    // Initialize Logging - File Only
    let file_appender = tracing_appender::rolling::never("logs", "openscholar.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_ansi(false) // Disable colors for file logs
        .init();

    tracing::info!("\n\n--- New Execution Started: {} ---", chrono::Local::now());

    // Auto-create .env if it doesn't exist
    if !std::path::Path::new(".env").exists() && std::path::Path::new(".env.example").exists() {
        tracing::info!("Creating .env from .env.example...");
        std::fs::copy(".env.example", ".env")?;
    }

    let args = Args::parse();

    if args.title.is_none() && args.author.is_none() && args.university.is_none() {
        tracing::error!("Please provide at least a --title, --author, or --university.");
        tracing::info!("Use --help for more information.");
        return Ok(());
    }

    let ss_api_key = env::var("SEMANTIC_SCHOLAR_API_KEY").ok();
    let email = env::var("OPENALEX_EMAIL").ok();
    let download_dir = env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "downloads".to_string());
    
    // Ensure download directory exists
    if !std::path::Path::new(&download_dir).exists() {
        tracing::info!("Creating download directory: {}", download_dir);
        std::fs::create_dir_all(&download_dir)?;
    }

    // 1. Discovery (Layer 1)
    tracing::info!("--- Step 1: Discovery (Parallel) ---");
    let query = DiscoveryQuery {
        title: args.title.clone(),
        author: args.author.clone(),
        university: args.university.clone(),
        category: args.category.clone(),
        limit: args.limit,
    };

    let orchestrator = DiscoveryOrchestrator::new(ss_api_key, email);
    let results = orchestrator.search_all(&query).await;
    tracing::info!("Found {} candidates from combined sources.", results.len());

    if results.is_empty() {
        tracing::warn!("No papers found in discovery phase.");
        return Ok(());
    }

    // 2. Resolution (Layer 2)
    tracing::info!("--- Step 2: Fuzzy Resolution ---");
    let search_title = args.title.as_deref().unwrap_or("");
    let matches = Resolver::resolve(search_title, results, args.threshold);
    let all_sorted = Resolver::sort_by_similarity(matches);

    // Filter: Only show papers that are Open Access AND have a PDF URL
    let (sorted_matches, unavailable_matches): (Vec<_>, Vec<_>) = all_sorted.into_iter()
        .partition(|(p, _)| p.is_oa && p.pdf_url.is_some());

    // Save unavailable papers
    if !unavailable_matches.is_empty() {
        let unavailable_papers: Vec<_> = unavailable_matches.into_iter().map(|(p, _)| p).collect();
        let downloader = crate::layers::download::Downloader::new(download_dir.clone());
        if let Err(e) = downloader.save_unavailable(&query, unavailable_papers).await {
            tracing::warn!("Failed to save unavailable papers: {}", e);
        }
    }
    
    if sorted_matches.is_empty() {
        tracing::warn!("No downloadable (Open Access + PDF) matches found within threshold {}.", args.threshold);
        return Ok(());
    }

    // Interactive Selection
    println!("\n--- candidates found ---");
    // Interactive Selection
    println!("\n--- candidates found ---");
    for (i, (paper, dist)) in sorted_matches.iter().enumerate().take(args.limit) {
        let source_hint = if paper.arxiv_id.is_some() { "[ArXiv]" } else if paper.open_alex_id.is_some() { "[OpenAlex]" } else { "[SemanticScholar]" };
        let oa_status = if paper.is_oa { "Open Access" } else { "Closed Access" };
        println!("[{}] {} (Dist: {}) {} - {}", i + 1, paper.title, dist, source_hint, oa_status);
    }

    println!("\nEnter numbers to download (e.g., '1', '1,3'), 'all' for top 10, or 'q' to quit:");
    
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let input = input.trim();

    if input.eq_ignore_ascii_case("q") {
        tracing::info!("Exiting.");
        return Ok(());
    }

    let indices: Vec<usize> = if input.eq_ignore_ascii_case("all") {
        (0..sorted_matches.len().min(args.limit)).collect()
    } else {
        input.split(',')
            .filter_map(|s| s.trim().parse::<usize>().ok())
            .map(|i| i.wrapping_sub(1)) // Convert 1-based to 0-based
            .filter(|&i| i < sorted_matches.len())
            .collect()
    };

    if indices.is_empty() {
        tracing::warn!("No valid selection made.");
        return Ok(());
    }

    // 4. Download (Layer 4)
    tracing::info!("--- Step 3: Download ---");
    let downloader = Downloader::new(download_dir);
    
    for idx in indices {
        let (paper, _) = &sorted_matches[idx];
        
        // 3. Legality Check (Layer 3) - Late binding check
        if !crate::layers::legality::LegalityChecker::is_legally_downloadable(paper) {
            tracing::warn!("Skipping '{}': Not Open Access.", paper.title);
            continue;
        }

        if paper.pdf_url.is_none() {
             tracing::warn!("Skipping '{}': No PDF URL available.", paper.title);
             continue;
        }

        tracing::info!("Downloading: {}", paper.title);
        match downloader.download_paper(paper).await {
            Ok(path) => tracing::info!("Success! Saved to: {:?}", path),
            Err(e) => tracing::error!("Failed to download '{}': {}", paper.title, e),
        }
    }

    Ok(())
}
