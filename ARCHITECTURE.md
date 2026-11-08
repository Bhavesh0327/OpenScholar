# Architecture

OpenScholar is a Rust CLI and library that finds and downloads open-access papers. It is also the academic-retrieval component of the Opportunity Intelligence Engine; the spec lives outside this repo. Current behaviour and the gap register are in [docs/BASELINE.md](docs/BASELINE.md).

## Layout

| Path | Role |
| --- | --- |
| `src/lib.rs` | Library root; exposes `layers`. |
| `src/main.rs` | Thin, interactive CLI over the library (clap args → discover → resolve → prompt → download). |
| `src/layers/mod.rs` | `PaperMetadata` (shared record) and `DiscoveryQuery`. |
| `src/layers/discovery.rs` | Semantic Scholar, arXiv and OpenAlex clients, plus `DiscoveryOrchestrator::search_all` (concurrent `tokio::join!`). |
| `src/layers/resolution.rs` | Levenshtein title matching and sorting. |
| `src/layers/legality.rs` | Open-access gate. |
| `src/layers/download.rs` | PDF download, `metadata.json`, `manifest.json`, `unavailable.json`. |
| `tests/provider_fixtures.rs` | Fixture tests; `#[ignore = "GAP-xx"]` marks open gaps. |
| `tests/fixtures/` | Recorded API responses (the SS search fixture is synthetic). |
| `tests/network.rs` | Mock-server (`wiremock`) tests for clients, orchestrator and downloader. |
| `tests/common/mod.rs` | Shared test helpers (`fixture`, `query`, `bare_paper`, `temp_dir`). |
| `examples/baseline.rs` | Opt-in live run; writes coverage, latency and duplicate stats as JSON. |

## Key pointers

- Each client has a `pub base_url`; override it in tests instead of adding config types.
- `wiremock` is pinned to `=0.6.2` because 0.6.5 needs edition 2024 and the local toolchain is Rust 1.82.
- Use `tokio::fs::write` (or call `flush()`) for every file write. A dropped `tokio::fs::File` may still have writes in flight (GAP-17).
- Each provider client is split into `build_url` (pure), `search` (network) and `parse_*` (pure). Test parsing and URL building against fixtures without touching the network.
- arXiv parsing: quick-xml 0.39 emits `Event::GeneralRef` for entities and splits the surrounding text, so text is accumulated per field and `trim_text` is off. Fields are trimmed when the entry closes.
- Semantic Scholar sends `abstract` (not `abstractText`). OpenAlex sends abstracts as `abstract_inverted_index`, which is rebuilt into text by sorting word positions.
- Anonymous Semantic Scholar requests are rate-limited aggressively (HTTP 429 seen on the first call).
- `Downloader::storage_id` picks the download directory: DOI, then arXiv ID, then S2 ID, then `unknown_id`.
- Errors and logs go to `logs/openscholar.log` only, not the console.

## Roadmap (Phase 0/1, Rust side)

0.2 parser fixes (done) → 0.3 base URLs, mock-server tests and live baseline (done) → 1.1 canonical IDs and dedup → 1.2 per-source status, rate limits, 429 retry → 1.3 exact-title vs topic mode → 1.4 `--json --non-interactive --metadata-only` and `discover()` → 1.5 download hardening.
