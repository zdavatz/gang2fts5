# gang2fts5

Volltextsuche und KI-gestützte Fragen über die Vorträge der [Ganglion-Organisation](https://ganglion.ch) — Themen wie ADHS, Psychiatrie, Erziehung, Sucht und psychische Gesundheit.

## Features

- **PDF-Download & Indexierung** — Lädt alle Vortrags-PDFs von ganglion.ch herunter, extrahiert den Text und indexiert ihn in SQLite mit FTS5
- **Volltextsuche** — Schnelle Suche über 367 Vorträge mit Snippet-Highlighting
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

# Deploy: build, index and scp binary + DB to remote server
./target/release/gang2fts5 deploy
```

### Deploy-Konfiguration

Erstelle eine Datei `deploy.conf` (wird nicht committed):

```
DEPLOY_TARGET=user@host:/path/to/deploy/
```

Der Deploy baut ein statisches Binary (musl target `x86_64-unknown-linux-musl`), das auf jedem x86_64-Linux ohne Abhängigkeiten läuft. Voraussetzung: musl-Toolchain (konfiguriert in `.cargo/config.toml` via `CC_x86_64_unknown_linux_musl`).

### DB aktualisieren (neue Vorträge)

```bash
# 1. Neue PDFs herunterladen (bestehende werden übersprungen)
bash download_pdfs.sh

# 2. Neue PDFs in die DB indexieren (bestehende werden übersprungen)
./target/release/gang2fts5 index
```

### Nur DB auf den Server kopieren

```bash
scp ganglion.db user@host:/path/to/deploy/
```

### Nur Binary auf den Server kopieren

```bash
scp target/x86_64-unknown-linux-musl/release/gang2fts5 user@host:/path/to/deploy/
```

Nach dem Kopieren muss der Server-Prozess neu gestartet werden:

```bash
pkill -f "gang2fts5 serve"
cd /path/to/deploy
nohup ./gang2fts5 serve > /tmp/gang2fts5.log 2>&1 &
```

### Apache Reverse Proxy (SSL)

Die Datei `gang2fts5-ssl.conf` enthält die Apache-Konfiguration für `ki.ganglion.ch` mit SSL und Reverse Proxy auf `http://localhost:3000`.

```bash
# Apache-Module aktivieren
sudo a2enmod proxy proxy_http rewrite ssl

# Konfiguration verlinken und aktivieren
sudo cp gang2fts5-ssl.conf /etc/apache2/sites-available/
sudo a2ensite gang2fts5-ssl

# SSL-Zertifikat mit Let's Encrypt erstellen
sudo certbot certonly --apache -d ki.ganglion.ch

# Apache neu starten
sudo systemctl restart apache2
```

## Weiterführende Informationen

- [Ganglion-Podcast auf Spotify](https://open.spotify.com/show/67sgy1aLTLXKCWkmgoqJ46)
- [YouTube-Kanal @udavatz](https://www.youtube.com/@udavatz)

## Lizenz

GPL-3.0
