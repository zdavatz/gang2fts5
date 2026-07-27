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
./target/release/gang2fts5 update                # download new PDFs + index into DB
./target/release/gang2fts5 deploy                # build musl binary, index, scp binary+DB to remote
bash download_pdfs.sh                            # download all PDFs from ganglion.ch
```

## Architecture

- **src/main.rs** — CLI + web server (axum), subcommands: `index`, `search`, `serve`, `update`, `deploy`
  - `init_db()` — schema with `documents` table (filename, title, date, audio_url, content) + FTS5 virtual table with content-sync triggers, handles migrations
  - `extract_pdf_text()` — PDF text extraction with `pdf-extract`, wrapped in `catch_unwind` for crash resilience
  - `extract_audio_url()` — regex scan of raw PDF bytes for audio links (adhs.expert, schizoud.wordpress.com, etc.)
  - `index_pdfs()` — walks pdf/ dir, extracts text, inserts into SQLite (skips existing)
  - `populate_metadata()` — sets titles, dates, and audio URLs from `titles.rs` + PDF binary scan
  - `retrieve_context()` — FTS5 search to find relevant chunks for RAG
  - `build_grok_request()` — constructs streaming chat completion request for xAI API
  - `api_ask()` — SSE streaming endpoint: FTS5 retrieval → Grok streaming → token-by-token response
  - `format_text_html()` — shared text formatter: joins PDF lines into flowing text, bolds timestamps, bold+italic speakers, auto-linkifies URLs
  - `vortrag_page()` — detail page with formatted text, speaker/date header, audio+PDF links
- **src/titles.rs** — static metadata mapping (vortrag ID → title + date) for ~320 lectures, scraped from ganglion.ch
- **src/index.html** — SPA with search mode, ask mode (SSE streaming), markdown rendering, source links
- **download_pdfs.sh** — downloads all PDFs from ganglion.ch into pdf/

## Deploy

The `deploy` subcommand builds a static musl binary (`x86_64-unknown-linux-musl`), indexes PDFs, and scps binary + DB to the remote server configured in `deploy.conf` (gitignored). Uses `rustls` instead of OpenSSL. The musl-gcc path is set via `CC_x86_64_unknown_linux_musl` in `.cargo/config.toml`.

## Flyer Generator (`flyer/`)

Standalone crate — its own `Cargo.toml` carries an empty `[workspace]` table so it stays out of the root gang2fts5 package. Unrelated to the search app; it generates the one-page A4 German flyer (`flyer/educational_engineering.pdf`) for Dr. Davatz's paid teacher-training course "Weiterbildungskurs im Umgang mit ADHS- und ADS-Kindern und Jugendlichen" (target audience: state educators — teachers, kindergarten, after-school staff). "Educational Engineering" is only an English tagline in the header; the authoritative course content (dates, CHF 1'200 fee, registration by e-mail) comes from ganglion.ch `popup_kurse.php?kurs_id=47` — do not confuse it with the free relatives'/parents' group (`kurs_id=46`, Mondays, registration required).

```bash
cd flyer && cargo build --release && ./target/release/flyer educational_engineering.pdf
```

- **flyer/src/main.rs** — hand-rolled layout engine on `printpdf`:
  - `Doc::width()` — text measurement from `ttf-parser` glyph advances; unkerned, which matches how printpdf renders, so measured width equals drawn width
  - `Doc::parse()` — `**bold**` / `*italic*` inline markup → styled runs
  - `Doc::words()` — splits runs into whitespace-delimited words that may mix styles, so punctuation stays glued across a style boundary (`**Bauart**:` must not render as `Bauart :`)
  - `Doc::para()` — ragged-right wrapping; returns the last baseline so blocks stack without hardcoded offsets
  - `Doc::lines()` — line count, used to pre-compute the facts-box height before its background is drawn
  - `Doc::link()` / `Doc::text_link()` — clickable URI annotations (`LinkAnnotation` + `Actions::uri`), hit box grown to the ascender/descender around the baseline
  - The "Auf einen Blick" box rows are `(label, [(line, is_address)])` (Für wen / Leitung / Daten / Ort / Kosten); address lines get `MAP_URL`. The band's e-mail and the footer's four segments (address→`MAP_URL`, phone→`TEL_URL`, e-mail→`MAIL_URL`, web→`WEB_URL`) are each drawn with `text_link`; the footer contact line is centred by pre-measuring total width and stepping segment-by-segment
  - Coordinates are mm with y measured from the page top, flipped to PDF's bottom-left origin at draw time (`PAGE_H - y`)
- printpdf 0.7 also writes a stray `/Subtype /Link` dict into `/Resources` (a broken `From<LinkAnnotationList>` impl). It carries no `/URI` and is not referenced from the page's `/Annots`, so viewers ignore it — the real annotations are built correctly in `pdf_document.rs`. Verify links with:
  `python3 -c "import pikepdf; [print(a.get('/A',{}).get('/URI')) for a in pikepdf.open('flyer/educational_engineering.pdf').pages[0]['/Annots']]"`
- Fonts embedded from `/usr/share/fonts/dejavu` (Sans, Bold, Oblique) for full umlaut coverage. Not subsetted, hence the ~2 MB output.

## Key Dependencies

- `rusqlite` (bundled SQLite with FTS5), `pdf-extract`, `axum`, `tokio`, `reqwest` (streaming, rustls-tls), `clap`, `regex`, `async-stream`
- `flyer/` only: `printpdf`, `ttf-parser`

## Environment

- `XAI_API_KEY` — required for `serve` command (Grok API)

## DB Schema

`documents`: id, filename, title, date, audio_url, content
`documents_fts`: FTS5 virtual table synced via triggers (filename, title, content)

## URL Routes

- `/` — search/ask SPA
- `/vortrag/:id` — detail page with formatted transcript (e.g. `/vortrag/580`)
- `/api/search?q=...` — JSON search results
- `/api/ask` — POST, SSE streaming RAG response
