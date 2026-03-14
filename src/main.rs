mod titles;

use anyhow::{Context, Result};
use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    response::{Html, Response},
    routing::{get, post},
    Json, Router,
};
use clap::{Parser, Subcommand};
use futures::StreamExt;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Parser)]
#[command(name = "gang2fts5", about = "SQLite FTS5 search over Ganglion PDFs")]
struct Cli {
    /// Path to the SQLite database
    #[arg(short, long, default_value = "ganglion.db")]
    db: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Index all PDFs from the pdf/ directory into the database
    Index {
        /// Directory containing PDF files
        #[arg(short, long, default_value = "pdf")]
        pdf_dir: String,
    },
    /// Search the indexed PDFs
    Search {
        /// FTS5 search query
        query: Vec<String>,
    },
    /// Start web GUI
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },
    /// Build database and deploy binary + DB to remote server
    Deploy {
        /// Directory containing PDF files
        #[arg(short, long, default_value = "pdf")]
        pdf_dir: String,
    },
}

fn init_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS documents (
            id INTEGER PRIMARY KEY,
            filename TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL DEFAULT '',
            date TEXT NOT NULL DEFAULT '',
            audio_url TEXT NOT NULL DEFAULT '',
            content TEXT NOT NULL
        );",
    )?;

    // Migrations for existing databases
    for col in &["title", "date", "audio_url"] {
        let has_col: bool = conn
            .prepare(&format!(
                "SELECT COUNT(*) FROM pragma_table_info('documents') WHERE name='{}'",
                col
            ))?
            .query_row([], |row| row.get::<_, i64>(0))
            .map(|c| c > 0)?;

        if !has_col {
            conn.execute_batch(&format!(
                "ALTER TABLE documents ADD COLUMN {} TEXT NOT NULL DEFAULT ''",
                col
            ))?;
        }
    }

    let fts_needs_rebuild = conn
        .prepare("SELECT sql FROM sqlite_master WHERE name='documents_fts'")
        .and_then(|mut s| s.query_row([], |row| row.get::<_, String>(0)))
        .map(|sql| !sql.contains("title"))
        .unwrap_or(true);

    if fts_needs_rebuild {
        conn.execute_batch(
            "DROP TABLE IF EXISTS documents_fts;
             CREATE VIRTUAL TABLE documents_fts USING fts5(
                 filename,
                 title,
                 content,
                 content=documents,
                 content_rowid=id
             );
             INSERT INTO documents_fts(documents_fts) VALUES('rebuild');",
        )?;
    }

    conn.execute_batch(
        "DROP TRIGGER IF EXISTS documents_ai;
        DROP TRIGGER IF EXISTS documents_ad;
        DROP TRIGGER IF EXISTS documents_au;
        CREATE TRIGGER documents_ai AFTER INSERT ON documents BEGIN
            INSERT INTO documents_fts(rowid, filename, title, content)
            VALUES (new.id, new.filename, new.title, new.content);
        END;
        CREATE TRIGGER documents_ad AFTER DELETE ON documents BEGIN
            INSERT INTO documents_fts(documents_fts, rowid, filename, title, content)
            VALUES ('delete', old.id, old.filename, old.title, old.content);
        END;
        CREATE TRIGGER documents_au AFTER UPDATE ON documents BEGIN
            INSERT INTO documents_fts(documents_fts, rowid, filename, title, content)
            VALUES ('delete', old.id, old.filename, old.title, old.content);
            INSERT INTO documents_fts(rowid, filename, title, content)
            VALUES (new.id, new.filename, new.title, new.content);
        END;",
    )?;

    Ok(())
}

fn extract_audio_url(pdf_path: &Path) -> Option<String> {
    let data = std::fs::read(pdf_path).ok()?;
    let text = String::from_utf8_lossy(&data);
    // Match any m4a/mp3 URL (adhs.expert, schizoud.wordpress.com, etc.)
    let re = regex::Regex::new(r"https?://[^\s)\x22<>]+\.(m4a|mp3)").ok()?;
    re.find(&text).map(|m| m.as_str().to_string())
}

fn populate_metadata(conn: &Connection, pdf_dir: &str) -> Result<()> {
    let meta_map = titles::get_metadata();
    let mut updated = 0;

    for (id, meta) in &meta_map {
        let filename = format!("vortrag_{}.pdf", id);
        let changed = conn.execute(
            "UPDATE documents SET title = ?1, date = ?2 WHERE filename = ?3 AND (title != ?1 OR date != ?2)",
            rusqlite::params![meta.title, meta.date, filename],
        )?;
        if changed > 0 {
            updated += 1;
        }
    }

    // Extract audio URLs from PDFs
    let mut audio_updated = 0;
    let pattern = format!("{}/*.pdf", pdf_dir);
    let paths: Vec<_> = glob::glob(&pattern)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|r| r.ok())
        .collect();

    for path in &paths {
        let filename = path.file_name().unwrap().to_string_lossy().to_string();
        if let Some(audio_url) = extract_audio_url(path) {
            let changed = conn.execute(
                "UPDATE documents SET audio_url = ?1 WHERE filename = ?2 AND audio_url != ?1",
                rusqlite::params![audio_url, filename],
            )?;
            if changed > 0 {
                audio_updated += 1;
            }
        }
    }

    if updated > 0 || audio_updated > 0 {
        conn.execute_batch("INSERT INTO documents_fts(documents_fts) VALUES('rebuild')")?;
        println!("Updated {} titles/dates, {} audio URLs, FTS index rebuilt", updated, audio_updated);
    }

    Ok(())
}

fn extract_pdf_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let result = std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem(&bytes));
    match result {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) => Err(anyhow::anyhow!("pdf extraction error: {}", e)),
        Err(_) => Err(anyhow::anyhow!("pdf extraction panicked")),
    }
}

fn index_pdfs(conn: &Connection, pdf_dir: &str) -> Result<()> {
    let meta_map = titles::get_metadata();
    let pattern = format!("{}/*.pdf", pdf_dir);
    let paths: Vec<_> = glob::glob(&pattern)
        .context("invalid glob pattern")?
        .filter_map(|r| r.ok())
        .collect();

    if paths.is_empty() {
        anyhow::bail!("No PDF files found in '{}'", pdf_dir);
    }

    println!("Found {} PDF files", paths.len());

    let mut indexed = 0;
    let mut skipped = 0;
    let mut failed = 0;

    for path in &paths {
        let filename = path.file_name().unwrap().to_string_lossy().to_string();

        let exists: bool = conn.query_row(
            "SELECT COUNT(*) > 0 FROM documents WHERE filename = ?1",
            [&filename],
            |row| row.get(0),
        )?;

        if exists {
            skipped += 1;
            continue;
        }

        let vortrag_id = filename
            .strip_prefix("vortrag_")
            .and_then(|s| s.strip_suffix(".pdf"))
            .and_then(|s| s.parse::<u32>().ok());

        let meta = vortrag_id.and_then(|id| meta_map.get(&id));
        let title = meta.map(|m| m.title).unwrap_or("");
        let date = meta.map(|m| m.date).unwrap_or("");

        match extract_pdf_text(path) {
            Ok(text) => {
                if text.trim().is_empty() {
                    eprintln!("  [SKIP] {} - no text content", filename);
                    failed += 1;
                    continue;
                }
                conn.execute(
                    "INSERT INTO documents (filename, title, date, content) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![filename, title, date, text],
                )?;
                indexed += 1;
                let display = if title.is_empty() { &filename } else { title };
                println!("  [OK] {} ({} chars)", display, text.len());
            }
            Err(e) => {
                eprintln!("  [FAIL] {}: {}", filename, e);
                failed += 1;
            }
        }
    }

    println!(
        "\nDone: {} indexed, {} skipped (already in db), {} failed",
        indexed, skipped, failed
    );
    Ok(())
}

fn cli_search(conn: &Connection, query: &str) -> Result<()> {
    let mut stmt = conn.prepare(
        "SELECT d.filename, d.title,
                snippet(documents_fts, 2, '>>>', '<<<', '...', 64) as snippet,
                rank
         FROM documents_fts
         JOIN documents d ON d.id = documents_fts.rowid
         WHERE documents_fts MATCH ?1
         ORDER BY rank
         LIMIT 20",
    )?;

    let rows = stmt.query_map([query], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, f64>(3)?,
        ))
    })?;

    let mut count = 0;
    for row in rows {
        let (filename, title, snippet, rank) = row?;
        count += 1;
        let display = if title.is_empty() { filename } else { title };
        println!("--- [{:.2}] {} ---", rank, display);
        println!("{}\n", snippet.trim());
    }

    if count == 0 {
        println!("No results found.");
    } else {
        println!("({} results)", count);
    }

    Ok(())
}

fn vortrag_id_from_filename(filename: &str) -> String {
    filename
        .strip_prefix("vortrag_")
        .and_then(|s| s.strip_suffix(".pdf"))
        .unwrap_or(filename)
        .to_string()
}

/// Format raw text into HTML: join lines into flowing text, bold timestamps, bold+italic speakers
fn format_text_html(text: &str) -> String {
    let normalized = regex::Regex::new(r" {2,}")
        .unwrap()
        .replace_all(text, " ");

    let timestamp_re = regex::Regex::new(r"^\[?\d{2}:\d{2}").unwrap();
    let speaker_re =
        regex::Regex::new(r"^(Dr\.|Prof\.|Dipl\.|Bemerkung|Audio\b|Referent)").unwrap();

    let lines: Vec<&str> = normalized.lines().collect();
    let mut joined = String::new();

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if timestamp_re.is_match(trimmed) {
            if !joined.is_empty() {
                joined.push_str("\n\n");
            }
            joined.push_str(trimmed);
            joined.push('\n');
            continue;
        }
        if speaker_re.is_match(trimmed) {
            if !joined.is_empty() && !joined.ends_with('\n') {
                joined.push_str("\n\n");
            }
            joined.push_str(trimmed);
            joined.push('\n');
            continue;
        }
        if !joined.is_empty() && !joined.ends_with('\n') && !joined.ends_with(' ') {
            joined.push(' ');
        }
        joined.push_str(trimmed);
    }

    let collapsed = regex::Regex::new(r"\n{3,}")
        .unwrap()
        .replace_all(&joined, "\n\n");

    // Extract URLs before HTML-escaping, replace with placeholders
    let url_re = regex::Regex::new(r#"(https?://[^\s<>"]+)"#).unwrap();
    let mut urls: Vec<String> = Vec::new();
    let with_placeholders = url_re.replace_all(&collapsed, |caps: &regex::Captures| {
        let url = caps[1].to_string();
        let idx = urls.len();
        urls.push(url);
        format!("__URL_PLACEHOLDER_{}__", idx)
    });

    let escaped = with_placeholders
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");

    // Bold timestamps, bold+italic speakers
    let ts_re = regex::Regex::new(r#"(?m)^\[(\d{2}:\d{2}:\d{2}\.\d{3})\] - (.+)$"#).unwrap();
    let formatted = ts_re.replace_all(&escaped, "<b>[$1]</b> - <b><i>$2</i></b>");

    let sp_re = regex::Regex::new(r#"(?m)^((?:Dr\.med\.|Prof\.|Dipl\.)[^\n(]+\(\d{2}:\d{2}\))"#).unwrap();
    let formatted = sp_re.replace_all(&formatted, "<b><i>$1</i></b>");

    let formatted = formatted
        .replace("\n\n", "<br><br>")
        .replace('\n', "<br>");

    let br_re = regex::Regex::new(r"(<br>){3,}").unwrap();
    let mut result = br_re.replace_all(&formatted, "<br><br>").to_string();

    // Restore URL placeholders as clickable links
    for (idx, url) in urls.iter().enumerate() {
        let placeholder = format!("__URL_PLACEHOLDER_{}__", idx);
        let link = format!(
            r#"<a href="{}" target="_blank" rel="noopener" style="color:#4a90d9">{}</a>"#,
            url, url
        );
        result = result.replace(&placeholder, &link);
    }

    result
}

// --- Web GUI ---

struct SharedState {
    db: Mutex<Connection>,
    http: reqwest::Client,
    api_key: String,
}

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
}

#[derive(Serialize)]
struct SearchResult {
    filename: String,
    vortrag_id: String,
    title: String,
    date: String,
    audio_url: String,
    snippet: String,
    rank: f64,
}

#[derive(Deserialize)]
struct AskRequest {
    question: String,
}

#[derive(Serialize)]
struct SourceRef {
    title: String,
    filename: String,
    vortrag_id: String,
    audio_url: String,
}

// Retrieve relevant chunks from FTS5 for a question
// Returns (title, filename, snippet, audio_url)
fn retrieve_context(conn: &Connection, question: &str, limit: usize) -> Vec<(String, String, String, String)> {
    // Use the question words as FTS5 query
    let fts_query: String = question
        .split_whitespace()
        .filter(|w| w.len() > 2) // skip very short words
        .map(|w| w.replace(|c: char| !c.is_alphanumeric() && c != 'ä' && c != 'ö' && c != 'ü' && c != 'Ä' && c != 'Ö' && c != 'Ü' && c != 'ß', ""))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" OR ");

    if fts_query.is_empty() {
        return vec![];
    }

    let mut stmt = conn
        .prepare(
            "SELECT d.title, d.filename,
                    snippet(documents_fts, 2, '', '', '...', 128) as snippet,
                    d.audio_url
             FROM documents_fts
             JOIN documents d ON d.id = documents_fts.rowid
             WHERE documents_fts MATCH ?1
             ORDER BY rank
             LIMIT ?2",
        )
        .unwrap();

    stmt.query_map(rusqlite::params![fts_query, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
        ))
    })
    .unwrap()
    .filter_map(|r| r.ok())
    .collect()
}

fn build_grok_request(
    question: &str,
    context_chunks: &[(String, String, String, String)],
) -> serde_json::Value {
    let mut context = String::new();
    for (i, (title, _filename, snippet, _audio_url)) in context_chunks.iter().enumerate() {
        context.push_str(&format!(
            "\n--- Quelle {}: \"{}\" ---\n{}\n",
            i + 1,
            title,
            snippet
        ));
    }

    let system_prompt = "Du bist ein hilfreicher Assistent, der Fragen basierend auf Vorträgen \
        der Ganglion-Organisation beantwortet. Diese Vorträge behandeln Themen wie ADHS, \
        Psychiatrie, Erziehung, Sucht und psychische Gesundheit. Die Inhalte sind evidenzbasiert \
        und stammen von Dr.med. Ursula Davatz und weiteren Fachpersonen.\n\
        Beantworte die Frage basierend auf den bereitgestellten Textabschnitten. \
        Antworte auf Deutsch. Verweise auf die Quellen (Vortragstitel) in deiner Antwort. \
        Füge KEINE Disclaimer hinzu wie \"dies ist nicht evidenzbasiert\" oder \
        \"konsultieren Sie weitere Quellen\". \
        Erwähne NIEMALS den Podcast, YouTube, Spotify oder sonstige externe Links. \
        Das wird separat angezeigt.";

    let user_msg = format!(
        "Hier sind relevante Textabschnitte aus Vorträgen:\n{}\n\nFrage: {}",
        context, question
    );

    serde_json::json!({
        "model": "grok-3-mini-fast",
        "stream": true,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_msg }
        ],
        "temperature": 0.3
    })
}

async fn index_page() -> Html<&'static str> {
    Html(include_str!("index.html"))
}

async fn api_search(
    State(state): State<Arc<SharedState>>,
    Query(params): Query<SearchParams>,
) -> Json<Vec<SearchResult>> {
    let query = match params.q {
        Some(q) if !q.trim().is_empty() => q.trim().to_string(),
        _ => return Json(vec![]),
    };

    let conn = state.db.lock().unwrap();

    let fts_query = query
        .split_whitespace()
        .map(|w| {
            if w == "AND" || w == "OR" || w == "NOT" || w == "NEAR" || w.starts_with('"') {
                w.to_string()
            } else {
                w.replace('"', "").to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ");

    let mut stmt = conn
        .prepare(
            "SELECT d.filename, d.title, d.date, d.audio_url,
                    snippet(documents_fts, 2, '<mark>', '</mark>', '...', 64) as snippet,
                    rank
             FROM documents_fts
             JOIN documents d ON d.id = documents_fts.rowid
             WHERE documents_fts MATCH ?1
             ORDER BY rank
             LIMIT 50",
        )
        .unwrap();

    let results: Vec<SearchResult> = stmt
        .query_map([&fts_query], |row| {
            let fname: String = row.get(0)?;
            let vid = vortrag_id_from_filename(&fname);
            let raw_snippet: String = row.get(4)?;
            // Preserve <mark> tags: temporarily replace, format, restore
            let snippet = raw_snippet.replace("<mark>", "\x01MARK\x01").replace("</mark>", "\x01/MARK\x01");
            let snippet = format_text_html(&snippet);
            let snippet = snippet.replace("\x01MARK\x01", "<mark>").replace("\x01/MARK\x01", "</mark>");
            Ok(SearchResult {
                filename: fname,
                vortrag_id: vid,
                title: row.get(1)?,
                date: row.get(2)?,
                audio_url: row.get(3)?,
                snippet,
                rank: row.get(5)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect();

    Json(results)
}

async fn api_ask(
    State(state): State<Arc<SharedState>>,
    Json(req): Json<AskRequest>,
) -> Response<Body> {
    let question = req.question.trim().to_string();

    if question.is_empty() {
        let msg = r#"data: {"type":"error","content":"Bitte stelle eine Frage."}"#;
        return sse_response(futures::stream::once(async move {
            Ok::<_, std::convert::Infallible>(format!("{}\n\n", msg))
        }));
    }

    // Step 1: Retrieve relevant context from FTS5
    let chunks = {
        let conn = state.db.lock().unwrap();
        retrieve_context(&conn, &question, 10)
    };

    if chunks.is_empty() {
        let msg = r#"data: {"type":"error","content":"Keine relevanten Textabschnitte gefunden."}"#;
        return sse_response(futures::stream::once(async move {
            Ok::<_, std::convert::Infallible>(format!("{}\n\n", msg))
        }));
    }

    // Collect and deduplicate sources
    let mut seen = std::collections::HashSet::new();
    let sources: Vec<SourceRef> = chunks
        .iter()
        .map(|(title, filename, _, audio_url)| SourceRef {
            title: title.clone(),
            vortrag_id: vortrag_id_from_filename(filename),
            filename: filename.clone(),
            audio_url: audio_url.clone(),
        })
        .filter(|s| seen.insert(s.filename.clone()))
        .collect();

    // Step 2: Send sources first, then stream Grok response
    let sources_json = serde_json::to_string(&sources).unwrap();
    let request_body = build_grok_request(&question, &chunks);
    let client = state.http.clone();
    let api_key = state.api_key.clone();

    let stream = async_stream::stream! {
        // Send sources immediately
        yield Ok::<_, std::convert::Infallible>(
            format!("data: {{\"type\":\"sources\",\"sources\":{}}}\n\n", sources_json)
        );

        // Start streaming from Grok
        let resp = client
            .post("https://api.x.ai/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&request_body)
            .send()
            .await;

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let status = r.status();
                let body = r.text().await.unwrap_or_default();
                yield Ok(format!(
                    "data: {{\"type\":\"error\",\"content\":\"Grok API Fehler {}: {}\"}}\n\n",
                    status,
                    body.replace('"', "\\\"").chars().take(200).collect::<String>()
                ));
                return;
            }
            Err(e) => {
                yield Ok(format!(
                    "data: {{\"type\":\"error\",\"content\":\"Verbindungsfehler: {}\"}}\n\n",
                    e.to_string().replace('"', "\\\"")
                ));
                return;
            }
        };

        let mut byte_stream = resp.bytes_stream();
        let mut buffer = String::new();

        while let Some(chunk) = byte_stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(_) => break,
            };
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE lines from buffer
            while let Some(pos) = buffer.find('\n') {
                let line = buffer[..pos].trim().to_string();
                buffer = buffer[pos + 1..].to_string();

                if !line.starts_with("data: ") {
                    continue;
                }
                let data = &line[6..];
                if data == "[DONE]" {
                    yield Ok("data: {\"type\":\"done\"}\n\n".to_string());
                    return;
                }

                // Parse the SSE chunk to extract content delta
                if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
                    if let Some(content) = parsed["choices"][0]["delta"]["content"].as_str() {
                        if !content.is_empty() {
                            let escaped = serde_json::to_string(content).unwrap();
                            yield Ok(format!(
                                "data: {{\"type\":\"token\",\"content\":{}}}\n\n",
                                escaped
                            ));
                        }
                    }
                }
            }
        }
        yield Ok("data: {\"type\":\"done\"}\n\n".to_string());
    };

    sse_response(stream)
}

fn sse_response<S>(stream: S) -> Response<Body>
where
    S: futures::Stream<Item = std::result::Result<String, std::convert::Infallible>>
        + Send
        + 'static,
{
    Response::builder()
        .header("Content-Type", "text/event-stream")
        .header("Cache-Control", "no-cache")
        .header("Connection", "keep-alive")
        .body(Body::from_stream(stream))
        .unwrap()
}

async fn vortrag_page(
    State(state): State<Arc<SharedState>>,
    AxumPath(id_or_filename): AxumPath<String>,
) -> Html<String> {
    let conn = state.db.lock().unwrap();

    // Support both /vortrag/580 and /vortrag/vortrag_580.pdf
    let filename = if id_or_filename.ends_with(".pdf") {
        id_or_filename.clone()
    } else {
        // Could be "580" or "580/some-slug"
        let id_part = id_or_filename.split('/').next().unwrap_or(&id_or_filename);
        format!("vortrag_{}.pdf", id_part)
    };

    let result = conn.query_row(
        "SELECT filename, title, date, audio_url, content FROM documents WHERE filename = ?1",
        [&filename],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        },
    );

    let (filename, title, date, audio_url, content) = match result {
        Ok(r) => r,
        Err(_) => return Html("<h1>Vortrag nicht gefunden</h1>".to_string()),
    };

    // Extract vortrag ID for ganglion.ch link
    let vortrag_id = filename
        .strip_prefix("vortrag_")
        .and_then(|s| s.strip_suffix(".pdf"))
        .unwrap_or("");

    let pdf_url = format!(
        "https://ganglion.ch/html/php/download_vortrag.php?id={}&download=pdf",
        vortrag_id
    );
    let audio_link_html = if audio_url.is_empty() {
        String::new()
    } else {
        format!(
            r#"<a href="{}" target="_blank" rel="noopener">Audio anhören</a>"#,
            audio_url
        )
    };

    // Split off header (everything before first timestamp) — we show it in the page header
    let body_start_re = regex::Regex::new(r"(?m)^\[?\d{2}:\d{2}").unwrap();
    let header_text = match body_start_re.find(&content) {
        Some(m) => &content[..m.start()],
        None => "",
    };
    let body_text = match body_start_re.find(&content) {
        Some(m) => &content[m.start()..],
        None => &content,
    };

    // Extract speaker names from the header
    let speaker = header_text
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && !l.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false))
        .unwrap_or("")
        .replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");

    let formatted = format_text_html(body_text);

    let title_escaped = title.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");

    let html = format!(
        r##"<!DOCTYPE html>
<html lang="de">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{title} - Ganglion Vortrag</title>
<style>
  * {{ box-sizing: border-box; margin: 0; padding: 0; }}
  body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; background: #f5f5f5; color: #333; }}
  .container {{ max-width: 900px; margin: 0 auto; padding: 20px; }}
  .header {{ background: white; border-radius: 8px; padding: 20px 24px; margin-bottom: 20px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.1); border-left: 4px solid #4a90d9; }}
  .header h1 {{ font-size: 1.5em; margin-bottom: 8px; color: #333; }}
  .header-links {{ display: flex; gap: 16px; flex-wrap: wrap; }}
  .header-links a {{ color: #4a90d9; text-decoration: none; font-size: 0.9em; }}
  .header-links a:hover {{ text-decoration: underline; }}
  .content {{ background: white; border-radius: 8px; padding: 24px 28px; margin-bottom: 20px;
    box-shadow: 0 1px 3px rgba(0,0,0,0.1); line-height: 1.6; font-size: 0.95em;
    word-wrap: break-word; }}
</style>
</head>
<body>
<div class="container">
  <div class="header">
    <h1>{title}</h1>
    <div style="color:#555; margin-bottom:4px; font-size:0.95em; font-style:italic;">{speaker}</div>
    <div style="color:#888; margin-bottom:10px; font-size:0.9em;">{date}</div>
    <div class="header-links">
      {audio_link}
      <a href="{pdf_url}" target="_blank" rel="noopener">PDF herunterladen</a>
    </div>
  </div>
  <div class="content">{content}</div>
</div>
</body>
</html>"##,
        title = title_escaped,
        speaker = speaker,
        date = date,
        audio_link = audio_link_html,
        pdf_url = pdf_url,
        content = formatted,
    );

    Html(html)
}

async fn start_server(db_path: &str, port: u16) -> Result<()> {
    let api_key = std::env::var("XAI_API_KEY")
        .context("XAI_API_KEY environment variable not set")?;

    let conn = Connection::open(db_path)?;
    init_db(&conn)?;

    let state = Arc::new(SharedState {
        db: Mutex::new(conn),
        http: reqwest::Client::new(),
        api_key,
    });

    let app = Router::new()
        .route("/", get(index_page))
        .route("/api/search", get(api_search))
        .route("/api/ask", post(api_ask))
        .route("/vortrag/:id", get(vortrag_page))
        .with_state(state);

    let addr = format!("0.0.0.0:{}", port);
    println!("Starting web GUI at http://localhost:{}", port);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

fn main() -> Result<()> {
    // Use 8MB stack for worker threads (musl defaults to 128KB which is too small)
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(8 * 1024 * 1024)
        .build()?;

    runtime.block_on(async_main())
}

async fn async_main() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Commands::Index { pdf_dir } => {
            let conn = Connection::open(&cli.db)?;
            init_db(&conn)?;
            index_pdfs(&conn, pdf_dir)?;
            populate_metadata(&conn, pdf_dir)?;
        }
        Commands::Search { query } => {
            let conn = Connection::open(&cli.db)?;
            init_db(&conn)?;
            let q = query.join(" ");
            if q.is_empty() {
                anyhow::bail!("Please provide a search query");
            }
            cli_search(&conn, &q)?;
        }
        Commands::Serve { port } => {
            start_server(&cli.db, *port).await?;
        }
        Commands::Deploy { pdf_dir } => {
            // Build static musl release binary
            println!("Building static release binary (musl)...");
            let status = std::process::Command::new("cargo")
                .args(["build", "--release", "--target", "x86_64-unknown-linux-musl"])
                .status()
                .context("Failed to run cargo build")?;
            if !status.success() {
                anyhow::bail!("cargo build --release failed");
            }

            // Build database
            println!("Indexing PDFs into {}...", &cli.db);
            let conn = Connection::open(&cli.db)?;
            init_db(&conn)?;
            index_pdfs(&conn, pdf_dir)?;
            populate_metadata(&conn, pdf_dir)?;
            drop(conn);

            // Read deploy target from deploy.conf
            let conf_path = Path::new("deploy.conf");
            let conf = std::fs::read_to_string(conf_path)
                .context("Failed to read deploy.conf — create it with DEPLOY_TARGET=user@host:/path/")?;
            let target = conf
                .lines()
                .find_map(|line| line.strip_prefix("DEPLOY_TARGET="))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("DEPLOY_TARGET not found in deploy.conf"))?
                .to_string();

            // scp binary and database
            let binary = "target/x86_64-unknown-linux-musl/release/gang2fts5";
            println!("Deploying {} and {} to {}", binary, &cli.db, &target);

            let status = std::process::Command::new("scp")
                .args([binary, &cli.db, &target])
                .status()
                .context("Failed to run scp")?;
            if !status.success() {
                anyhow::bail!("scp failed");
            }

            println!("Deploy complete.");
        }
    }

    Ok(())
}
