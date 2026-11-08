# Architecture

OpenScholar is a Rust CLI and library that finds and downloads open-access papers. It is also the academic-retrieval component of the Opportunity Intelligence Engine; the spec lives outside this repo. Current behaviour and the gap register are in [docs/BASELINE.md](docs/BASELINE.md).

## Layout

| Path | Role |
| --- | --- |
| `src/lib.rs` | Library root; exposes `layers`. |
| `src/main.rs` | Thin CLI over the library: args → `discover` → select (prompt, or top N when unattended) → download → output. |
| `src/layers/mod.rs` | `PaperMetadata` (shared record), `SourceStatus` and `DiscoveryQuery`. |
| `src/layers/discovery.rs` | Semantic Scholar, arXiv and OpenAlex clients, plus `DiscoveryOrchestrator::search_all` (concurrent `tokio::join!`). |
| `src/layers/resolution.rs` | Levenshtein title matching on normalized titles (lowercase, punctuation stripped), and sorting. |
| `src/layers/legality.rs` | Open-access gate. |
| `src/layers/identity.rs` | ID normalization (`canonicalize`) and cross-provider merge (`dedupe`). |
| `src/layers/download.rs` | PDF download, `metadata.json`, `manifest.json`, `unavailable.json`. |
| `tests/provider_fixtures.rs` | Fixture tests; `#[ignore = "GAP-xx"]` marks open gaps. |
| `tests/fixtures/` | Recorded API responses (the SS search fixture is synthetic). |
| `tests/cli_contract.rs` | Runs the built binary: `--json` schema, stderr and exit codes. |
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
- All provider HTTP goes through `discovery::fetch`. It applies the per-client rate limit (SS 1/s, arXiv one request every 3 s, OpenAlex 10/s), uses a 10 s connect and 30 s total timeout, and makes up to 3 attempts on 429/5xx using `Retry-After` or 1 s/2 s backoff. Non-2xx responses become `HttpError { source, status }`.
- `search_all` returns `(papers, Vec<SourceStatus>)`, with one status per source: ok, result count before dedup, latency, HTTP status and error. Failures are never dropped silently.
- IDs are canonical at parse time: DOIs are lowercase with no `doi.org` prefix, arXiv IDs have no URL or version, and OpenAlex IDs are short (`W…`). An arXiv ID is derived from a `10.48550/arxiv.*` DOI.
- `PaperMetadata.id` takes the first available of `doi:`, `arxiv:`, `s2:`, `openalex:`. `sources` lists the providers that returned the paper. Both fields default when old JSON is read.
- `search_all` returns `identity::dedupe` output, which merges records that share any ID. Records are not merged on title alone; that's a known limit, chosen to avoid false merges.
- Download safety in `Downloader`:
  - Redirects are followed manually (at most 5), and every hop is checked: only http/https, and no localhost, loopback, private, link-local or ULA addresses unless `allow_private_hosts` is set (tests only).
  - Timeouts are 10 s to connect and 120 s total. `max_bytes` (default 100 MB) is checked against `Content-Length` and while streaming. The body must start with `%PDF-`.
  - The body streams to `.<id>.part` and is renamed to `<id>/paper.pdf` only on success, so a failure leaves no files.
- JSON files (`manifest.json`, `unavailable.json`, `metadata.json`) are written atomically via a temp file and rename. A file that won't parse is moved to `*.corrupt-<timestamp>`, never silently overwritten.
- Known limits:
  - A hostname that resolves to a private IP is not blocked (blocking it would need a custom DNS resolver).
  - The file manifest is safe for one process only. Concurrent workers need the database planned for Phase 1 on the Python side.
- `Downloader::storage_id` picks the download directory: DOI, then arXiv, then S2, then OpenAlex ID. Directory names now use lowercase DOIs.
- Two search modes, with no mode enum. With `--title`, results are filtered by Levenshtein distance against the title (`--threshold`). With `--topic` (and no title), nothing is filtered: topic is free text for SS and OpenAlex, and words joined with AND (`all:w1 AND all:w2`) for arXiv. Any one of title, topic, author, category or university is enough to run a search.
- `DiscoveryOrchestrator::discover(query, threshold)` returns `LiteratureResult { schema_version, papers, source_statuses, downloads }`. It's the single entry point for both the CLI and the library. Bump `SCHEMA_VERSION` in `layers/mod.rs` on any breaking JSON change.
- CLI modes:
  - `--json` prints only the `LiteratureResult` JSON to stdout and implies non-interactive.
  - `--non-interactive` downloads the top `--limit` downloadable papers without prompting.
  - `--metadata-only` skips downloads and writes nothing to `DOWNLOAD_DIR`.
  - With no flags, the CLI prompts as before.
- Exit codes: 2 for invalid arguments, 1 for a runtime error, 0 otherwise (including when some sources fail; check `source_statuses`). Warnings and errors go to stderr; detailed logs still go to `logs/openscholar.log`.
- `OPENSCHOLAR_SEMANTIC_SCHOLAR_URL`, `OPENSCHOLAR_ARXIV_URL` and `OPENSCHOLAR_OPENALEX_URL` override provider base URLs. They're used by the contract tests and can point at a proxy or cache.

## Roadmap (Phase 0/1, Rust side)

0.2 parser fixes (done) → 0.3 base URLs, mock-server tests and live baseline (done) → 1.1 canonical IDs and dedup (done) → 1.2 per-source status, rate limits, 429 retry (done) → 1.3 exact-title vs topic mode (done) → 1.4 `--json --non-interactive --metadata-only` and `discover()` (done) → 1.5 download hardening (done). The Rust side of Phase 1 is complete; next is the Python/FastAPI service, built against the `--json` contract.
