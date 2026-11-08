# OpenScholar Baseline (Phase 0)

Recorded 2026-11-08 against commit `38b8f12`, Rust 1.82. This documents how the tool **actually behaves today**, so later changes can be checked against it. Gap IDs match the `#[ignore = "GAP-.."]` tests in `tests/provider_fixtures.rs`.

## Running the baseline

```sh
cargo test                     # 6 unit + 15 fixture + 10 mock-server tests; must stay green
cargo test -- --ignored        # remaining known-gap tests; expected to FAIL until fixed
cargo run --example baseline   # live run against real APIs (opt-in, not CI)
```

Fixtures live in `tests/fixtures/`. The arXiv and OpenAlex fixtures are recorded API responses; `openalex_attention.json` has its large reference lists removed. `semantic_scholar_search.json` is **synthetic**, because anonymous Semantic Scholar requests returned HTTP 429 on the first call. `semantic_scholar_429.json` is a real 429 response body.

## Live baseline (2026-11-08, no SS key)

Raw output is in `docs/baseline-2026-11-08.json`. Limit is 10 per source.

| Query | Semantic Scholar | arXiv | OpenAlex | Exact duplicate titles |
| --- | --- | --- | --- | --- |
| exact title | 429 | 10 (1.1 s) | 10 (4.7 s) | 5 / 20 |
| topic | 429 | 8 (0.5 s) | 10 (2.0 s) | 6 / 18 |
| author | 429 | 10 (0.7 s) | 10 (2.2 s) | 0 / 20 |
| title + university | 429 | **0** (0.5 s) | 10 (2.8 s) | 1 / 10 |

Notes:
- Anonymous Semantic Scholar fails on every query.
- arXiv has no affiliation data, so a university filter combined with an exact title phrase matches nothing.
- Duplicates are counted by exact lowercase title only, so the real duplicate rate is higher.

## Current behaviour

| Area | Behaviour |
| --- | --- |
| CLI input | `--title`, `--author`, `--university` (at least one is required). `--category` alone is rejected. `--threshold` (default 5) and `--limit` (default 10). |
| Startup side effects | Copies `.env.example` to `.env` in the cwd if `.env` is missing. Creates `logs/` and the download dir in the cwd. |
| Output | All diagnostics, including validation errors, go **only** to `logs/openscholar.log`. Stdout shows only the candidate list and the prompt. A validation failure prints nothing and exits 0. |
| Discovery | Queries Semantic Scholar, arXiv and OpenAlex concurrently. A provider failure is logged and dropped, and results are concatenated without deduplication. |
| Semantic Scholar | All query fields are joined into one free-text `query`. The limiter allows 400 req/s. There is no retry, and a 429 means zero SS results. |
| arXiv | The title becomes the exact phrase `ti:"…"`, and category and university are searched with `all:"…"`. Uses plain `http://`. No limiter. Every result gets `is_oa = true`. |
| OpenAlex | University maps to `raw_affiliation_strings.search`, other fields to `search=`. `mailto` is not URL-encoded. No limiter. |
| Resolution | Case-sensitive absolute Levenshtein distance against `--title`, threshold 5 characters. With no title, every result passes with distance 0. |
| Selection | Interactive stdin prompt (`1,3`, `all`, `q`), so unattended runs are impossible. |
| Download | `reqwest::Client::new()`: no timeout, no size cap, no content-type or `%PDF` check, and redirects to any host are followed. The file is created before streaming, so a failure leaves a partial `paper.pdf`. |
| Storage | `<DOWNLOAD_DIR>/<storage_id>/{paper.pdf,metadata.json}`, plus `manifest.json` and `unavailable.json` updated by read-modify-write. A corrupt JSON file is silently replaced with an empty one. |

## Gap register

| ID | Severity | Gap | Evidence |
| --- | --- | --- | --- |
| GAP-01 | High | **Fixed (0.2)** arXiv titles and abstracts containing XML entities are truncated to the last fragment (`R&amp;D Project` becomes `D Project`). | fixture test |
| GAP-02 | High | **Fixed (0.2)** Semantic Scholar abstracts are always `None` (serde expects `abstractText`, but the API sends `abstract`). | fixture test |
| GAP-03 | Medium | **Fixed (0.2)** OpenAlex `abstract_inverted_index` is not reconstructed. | fixture test |
| GAP-04 | High | No canonical IDs. arXiv ID is `http://arxiv.org/abs/1706.03762v7`, and DOIs come as bare mixed-case or `https://doi.org/` lowercase. No dedup in `search_all`. | fixture tests |
| GAP-05 | Medium | Papers with no DOI, arXiv or S2 ID all map to the `unknown_id/` directory and overwrite each other. The same paper from two providers lands in two different directories. | fixture tests |
| GAP-06 | High | Interactive-only CLI. No `--json`, `--non-interactive` or `--metadata-only`. | code |
| GAP-07 | Medium | Errors are invisible on the console, and the exit code is 0 on failure. | manual run |
| GAP-08 | Low | Category-only search is rejected. | manual run |
| GAP-09 | High | Title edit-distance filtering is unusable for topic search, and there is no exact-title vs topic mode. | code |
| GAP-10 | High | No 429 handling or backoff. The SS limit is 400/s; arXiv and OpenAlex have no limiter. Anonymous SS hit 429 on the first request. | live call |
| GAP-11 | High | No structured per-source status (count, latency, error kind). | code |
| GAP-12 | High | Download safety: no timeout, size cap, content-type or magic-byte check, host or redirect policy, or private-IP block. Partial files are left behind on failure. | code |
| GAP-13 | Medium | Manifest writes are not atomic, and corrupt files are reset to empty (data loss). | code |
| GAP-14 | Low | Startup writes `.env` and `logs/` into the cwd. | manual run |
| GAP-15 | Medium | Base URLs and HTTP clients are hard-coded, so network paths (retry, 429, download) cannot be tested against a mock server. | code |
| GAP-17 | High | **Fixed (0.3)** JSON files were written with `tokio::fs::File` and never flushed, so back-to-back read-modify-writes could read a partial file, reset it to empty and lose entries. Now uses `fs::write`, and the PDF stream is flushed. | flaky test, 7/30 runs |
| GAP-16 | Low | `categories` is always empty, there is no `year_from` or OA-only filter, `limit` is not clamped to provider maxima, and "candidates found" is printed twice. | code |

## Test seams added in Phase 0

- Each client has a `pub base_url` (production default set in `new()`), and the `DiscoveryOrchestrator` client fields are `pub`. Tests point them at a `wiremock` server.

These are pure functions with no behaviour change, used by the fixture tests:

- `SemanticScholarClient::{build_url, parse_response}`
- `ArxivClient::{build_url, parse_feed}`
- `OpenAlexClient::{build_url, parse_response}`
- `Downloader::storage_id`
- `src/lib.rs` exposes `layers`; `main.rs` is now a thin binary over the library crate.
