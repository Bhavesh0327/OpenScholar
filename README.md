# OpenScholar: Intelligent Research Discovery Engine

A Rust-based tool for the automated discovery, resolution, and downloading of open-access research papers. OpenScholar intelligently aggregates results from Semantic Scholar, arXiv, and OpenAlex to find the most relevant, legally downloadable PDF for your queries.

## Features

- **Layered Discovery**: Searches across Semantic Scholar, arXiv, and OpenAlex.
- **Smart Filtering**: Automatically filters results to show *only* downloadable papers (Open Access + PDF available).
- **Fuzzy Resolution**: Matches search results to your target title using Levenshtein distance.
- **Legality Enforcement**: Downloads only Open Access (OA) papers to ensure compliance.
- **Download Manifest**: Maintains a `manifest.json` tracking your entire collection.
- **Unavailability Tracking**: Logs separate details for relevant papers that could not be legally downloaded.
- **Automated Metadata**: Stores paper metadata (JSON) alongside the PDF in a structured directory.

## Setup

### Prerequisites
- [Rust](https://www.rust-lang.org/tools/install) (Compatible with 1.82.0+)

### Configuration
1. Clone the repository.
2. The tool will auto-create a `.env` file from `.env.example` on the first run.
3. Add your API keys/email to the `.env` file:
   - `OPENALEX_EMAIL`: Required for the OpenAlex "polite pool".
   - `SEMANTIC_SCHOLAR_API_KEY`: Highly recommended to avoid 429 Rate Limit errors.

## Usage

Run the project using Cargo with flags for title, author, category, or university:

```bash
# Search by title
cargo run -- --title "Attention Is All You Need"

# Search by author and category
cargo run -- --author "Vaswani" --category "cs.CL"

# Search by university with a limit
cargo run -- -u "New York University" -n 20

# Search by topic/category
cargo run -- -c "Machine Learning" -n 5

# View all options
cargo run -- --help
```

### CLI Options
- `-t, --title`: Title of the paper.
- `-a, --author`: Author name.
- `-c, --category`: Research category (e.g., `cs.AI`).
- `-u, --university`: University affiliation.
- `-n, --limit`: Maximum number of results to display (default: 10).
- `--threshold`: Custom Levenshtein distance for fuzzy matching (default: 5).

## Output Structure

Papers are downloaded to the directory specified in your `.env` file (default: `downloads/`).

```
downloads/
├── manifest.json          # Master list of all successful downloads
├── unavailable.json       # Record of papers found but not downloadable
├── doi_10.1234_.../       # Individual paper folder
│   ├── paper.pdf          # Full text
│   └── metadata.json      # Complete metadata
└── ...
```

### `manifest.json`
A simplified, flat list of all successfully downloaded papers, containing the title, author, year, and path for easy programmatic access.

### `unavailable.json`
A nested record of papers that were found by search but skipped (due to Closed Access or missing PDF). The structure follows your search priority:
`University` -> `Category` -> `Author` -> `Title`

Example:
```json
{
  "Stanford University": {
    "ML": [ ...papers... ]
  }
}
```

## Contributing

Interested in contributing? Please check out our [Contributing Guidelines](CONTRIBUTING.md) for more information.

## License
MIT
