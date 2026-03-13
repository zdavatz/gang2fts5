# gang2fts5

Volltextsuche und KI-gestützte Fragen über die Vorträge der [Ganglion-Organisation](https://ganglion.ch) — Themen wie ADHS, Psychiatrie, Erziehung, Sucht und psychische Gesundheit.

## Features

- **PDF-Download & Indexierung** — Lädt alle Vortrags-PDFs von ganglion.ch herunter, extrahiert den Text und indexiert ihn in SQLite mit FTS5
- **Volltextsuche** — Schnelle Suche über 326 Vorträge mit Snippet-Highlighting
- **KI-Antworten (RAG)** — Stellt eine Frage in natürlicher Sprache, die App sucht relevante Textabschnitte und lässt sie von Grok (xAI) beantworten, mit Streaming
- **Web-GUI** — Suchoberfläche mit Suchen- und Fragen-Modus, formatierte Vortrag-Detailseiten, Audio-Links und Quellenverweise
- **Audio-Links** — Direkte Links zu den Audio-Aufnahmen der Vorträge (von adhs.expert und schizoud.wordpress.com)
- **Formatierter Text** — Fliesstext mit fetten Timestamps, fett+kursiven Sprechernamen, automatisch verlinkte URLs

## Quickstart

```bash
# PDFs herunterladen
bash download_pdfs.sh

# Bauen
cargo build --release

# PDFs indexieren (extrahiert Text, Titel, Datum, Audio-URLs)
./target/release/gang2fts5 index

# CLI-Suche
./target/release/gang2fts5 search "ADHS Schule"

# Web-GUI starten (benötigt XAI_API_KEY für KI-Antworten)
export XAI_API_KEY="your-key"
./target/release/gang2fts5 serve
# -> http://localhost:3000
```

## Weiterführende Informationen

- [Ganglion-Podcast auf Spotify](https://open.spotify.com/show/67sgy1aLTLXKCWkmgoqJ46)
- [YouTube-Kanal @udavatz](https://www.youtube.com/@udavatz)

## Lizenz

GPL-3.0
