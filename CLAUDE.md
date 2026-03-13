# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

gang2fts5 is a Rust CLI + web app that downloads PDF lecture transcripts from ganglion.ch, indexes them in SQLite FTS5, and provides full-text search plus RAG-based Q&A via Grok (xAI API). Licensed under GPL-3.0.

## Build & Run

```bash
cargo build --release
./target/release/gang2fts5 index                # index PDFs from pdf/ into ganglion.db
./target/release/gang2fts5 search "ADHS"        # CLI search
./target/release/gang2fts5 serve                 # web GUI on port 3000 (needs XAI_API_KEY)
./target/release/gang2fts5 serve -p 8080         # custom port
bash download_pdfs.sh                            # download all PDFs from ganglion.ch
```

## Architecture

- **src/main.rs** — CLI + web server (axum), subcommands: `index`, `search`, `serve`
  - `init_db()` — schema with `documents` table (filename, title, date, audio_url, content) + FTS5 virtual table with content-sync triggers, handles migrations
  - `extract_pdf_text()` — PDF text extraction with `pdf-extract`, wrapped in `catch_unwind` for crash resilience
  - `extract_audio_url()` — regex scan of raw PDF bytes for `adhs.expert` audio links (m4a/mp3)
  - `index_pdfs()` — walks pdf/ dir, extracts text, inserts into SQLite (skips existing)
  - `populate_metadata()` — sets titles, dates, and audio URLs from `titles.rs` + PDF binary scan
  - `retrieve_context()` — FTS5 search to find relevant chunks for RAG
  - `build_grok_request()` — constructs streaming chat completion request for xAI API
  - `api_ask()` — SSE streaming endpoint: FTS5 retrieval → Grok streaming → token-by-token response
  - `vortrag_page()` — detail page rendering full text with JS-based search term highlighting
- **src/titles.rs** — static metadata mapping (vortrag ID → title + date) scraped from ganglion.ch
- **src/index.html** — SPA with search mode, ask mode (SSE streaming), markdown rendering, source links
- **download_pdfs.sh** — downloads all PDFs from ganglion.ch into pdf/

## Key Dependencies

- `rusqlite` (bundled SQLite with FTS5), `pdf-extract`, `axum`, `tokio`, `reqwest` (streaming), `clap`, `regex`, `async-stream`

## Environment

- `XAI_API_KEY` — required for `serve` command (Grok API)

## DB Schema

`documents`: id, filename, title, date, audio_url, content
`documents_fts`: FTS5 virtual table synced via triggers (filename, title, content)

## URL Routes

- `/` — search/ask SPA
- `/vortrag/:id?q=...` — detail page with highlighting (e.g. `/vortrag/580?q=ADHS`)
- `/api/search?q=...` — JSON search results
- `/api/ask` — POST, SSE streaming RAG response
