use openscholar::layers::{DiscoveryQuery, DownloadRecord, PaperMetadata};
use openscholar::layers::download::Downloader;
use openscholar::layers::legality::LegalityChecker;
use openscholar::layers::discovery::DiscoveryOrchestrator;
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

    /// Free-text topic search (no title matching)
    #[arg(long)]
    topic: Option<String>,

    /// Custom Levenshtein threshold for fuzzy matching
    #[arg(long, default_value_t = 5)]
    threshold: usize,

    /// Maximum number of results to return
    #[arg(short = 'n', long, default_value_t = 10)]
    limit: usize,

    /// Print a versioned JSON result to stdout (implies --non-interactive)
    #[arg(long)]
    json: bool,

    /// Download the top --limit downloadable papers without prompting
    #[arg(long)]
    non_interactive: bool,

    /// Only search; no downloads and no files written to DOWNLOAD_DIR
    #[arg(long)]
    metadata_only: bool,
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
    let unattended = args.json || args.non_interactive;

    if args.title.is_none() && args.author.is_none() && args.university.is_none() && args.category.is_none() && args.topic.is_none() {
        eprintln!("error: provide at least one of --title, --topic, --author, --category, --university (see --help)");
        std::process::exit(2);
    }

    let ss_api_key = env::var("SEMANTIC_SCHOLAR_API_KEY").ok().filter(|k| !k.is_empty());
    let email = env::var("OPENALEX_EMAIL").ok();
    let download_dir = env::var("DOWNLOAD_DIR").unwrap_or_else(|_| "downloads".to_string());

    let query = DiscoveryQuery {
        title: args.title.clone(),
        author: args.author.clone(),
        university: args.university.clone(),
        category: args.category.clone(),
        topic: args.topic.clone(),
        limit: args.limit,
    };

    let mut orchestrator = DiscoveryOrchestrator::new(ss_api_key, email);
    if let Ok(url) = env::var("OPENSCHOLAR_SEMANTIC_SCHOLAR_URL") { orchestrator.ss_client.base_url = url; }
    if let Ok(url) = env::var("OPENSCHOLAR_ARXIV_URL") { orchestrator.arxiv_client.base_url = url; }
    if let Ok(url) = env::var("OPENSCHOLAR_OPENALEX_URL") { orchestrator.open_alex_client.base_url = url; }

    let mut result = orchestrator.discover(&query, args.threshold).await;
    for status in &result.source_statuses {
        tracing::info!("Source {}: ok={} results={} latency={}ms error={:?}", status.source, status.ok, status.result_count, status.latency_ms, status.error);
        if !status.ok {
            eprintln!("warning: {} failed: {}", status.source, status.error.as_deref().unwrap_or("unknown error"));
        }
    }

    let (downloadable, unavailable): (Vec<_>, Vec<_>) = result.papers.iter().cloned()
        .partition(|p| p.is_oa && p.pdf_url.is_some());

    if !args.metadata_only {
        std::fs::create_dir_all(&download_dir)?;
        if let Err(e) = Downloader::new(download_dir.clone()).save_unavailable(&query, unavailable).await {
            tracing::warn!("Failed to save unavailable papers: {}", e);
        }
    }

    let selected: Vec<&PaperMetadata> = if args.metadata_only {
        Vec::new()
    } else if unattended {
        downloadable.iter().take(args.limit).collect()
    } else {
        if downloadable.is_empty() {
            eprintln!("No downloadable (Open Access + PDF) matches found.");
            return Ok(());
        }
        println!("\n--- candidates found ---");
        for (i, paper) in downloadable.iter().enumerate().take(args.limit) {
            let oa_status = if paper.is_oa { "Open Access" } else { "Closed Access" };
            println!("[{}] {} [{}] - {}", i + 1, paper.title, paper.sources.join(", "), oa_status);
        }

        println!("\nEnter numbers to download (e.g., '1', '1,3'), 'all' for top {}, or 'q' to quit:", args.limit);
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        let input = input.trim();

        if input.eq_ignore_ascii_case("q") {
            return Ok(());
        }
        if input.eq_ignore_ascii_case("all") {
            downloadable.iter().take(args.limit).collect()
        } else {
            input.split(',')
                .filter_map(|s| s.trim().parse::<usize>().ok())
                .filter_map(|i| downloadable.get(i.wrapping_sub(1)))
                .collect()
        }
    };

    let downloader = Downloader::new(download_dir);
    for paper in selected {
        if !LegalityChecker::is_legally_downloadable(paper) {
            continue;
        }
        tracing::info!("Downloading: {}", paper.title);
        let record = match downloader.download_paper(paper).await {
            Ok(path) => DownloadRecord { id: paper.id.clone(), path: Some(path.to_string_lossy().into_owned()), error: None },
            Err(e) => {
                eprintln!("warning: download failed for '{}': {}", paper.title, e);
                DownloadRecord { id: paper.id.clone(), path: None, error: Some(e.to_string()) }
            }
        };
        if !args.json {
            if let Some(path) = &record.path { println!("Saved: {}", path); }
        }
        result.downloads.push(record);
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if result.papers.is_empty() {
        eprintln!("No papers found.");
    }

    Ok(())
}
