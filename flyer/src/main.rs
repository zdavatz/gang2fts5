// Generates a one-page A4 flyer (German) for the "Educational Engineering" group.
use printpdf::path::{PaintMode, WindingOrder};
use printpdf::*;
use std::fs::File;
use std::io::BufWriter;

const PAGE_W: f32 = 210.0;
const PAGE_H: f32 = 297.0;
const M_L: f32 = 15.0;
const PT: f32 = 0.352_777_8; // pt -> mm

const GREEN: (f32, f32, f32) = (0.184, 0.420, 0.247);
const GREEN_BG: (f32, f32, f32) = (0.933, 0.957, 0.937);
const INK: (f32, f32, f32) = (0.110, 0.110, 0.110);
const GREY: (f32, f32, f32) = (0.353, 0.353, 0.353);
const RULE: (f32, f32, f32) = (0.847, 0.867, 0.851);

#[derive(Clone, Copy, PartialEq)]
enum S {
    Reg = 0,
    Bold = 1,
    Ital = 2,
}

/// A run of characters sharing one style.
struct Seg {
    text: String,
    style: S,
}

/// A whitespace-delimited word. It may mix styles ("**Bauart**:" -> bold + regular)
/// so punctuation hugging a style boundary stays glued to its word.
struct Word {
    segs: Vec<Seg>,
    w: f32,
}

struct Doc {
    layer: PdfLayerReference,
    refs: Vec<IndirectFontRef>,
    faces: Vec<ttf_parser::Face<'static>>,
}

impl Doc {
    fn width(&self, text: &str, s: S, size: f32) -> f32 {
        let f = &self.faces[s as usize];
        let upem = f.units_per_em() as f32;
        let mut w = 0.0f32;
        for ch in text.chars() {
            if let Some(g) = f.glyph_index(ch) {
                w += f.glyph_hor_advance(g).unwrap_or(0) as f32;
            }
        }
        w / upem * size * PT
    }

    fn fill(&self, c: (f32, f32, f32)) {
        self.layer
            .set_fill_color(Color::Rgb(Rgb::new(c.0, c.1, c.2, None)));
    }

    fn stroke(&self, c: (f32, f32, f32)) {
        self.layer
            .set_outline_color(Color::Rgb(Rgb::new(c.0, c.1, c.2, None)));
    }

    /// Draw a single string. `y` is the baseline, measured from the top of the page.
    fn text(&self, t: &str, x: f32, y: f32, size: f32, s: S, c: (f32, f32, f32)) {
        self.fill(c);
        self.layer
            .use_text(t, size, Mm(x), Mm(PAGE_H - y), &self.refs[s as usize]);
    }

    fn rect(&self, x: f32, y: f32, w: f32, h: f32, c: (f32, f32, f32)) {
        self.fill(c);
        let pts = vec![
            (Point::new(Mm(x), Mm(PAGE_H - y)), false),
            (Point::new(Mm(x + w), Mm(PAGE_H - y)), false),
            (Point::new(Mm(x + w), Mm(PAGE_H - y - h)), false),
            (Point::new(Mm(x), Mm(PAGE_H - y - h)), false),
        ];
        self.layer.add_polygon(Polygon {
            rings: vec![pts],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        });
    }

    fn hline(&self, x: f32, y: f32, w: f32, thick_pt: f32, c: (f32, f32, f32)) {
        self.stroke(c);
        self.layer.set_outline_thickness(thick_pt);
        self.layer.add_line(Line {
            points: vec![
                (Point::new(Mm(x), Mm(PAGE_H - y)), false),
                (Point::new(Mm(x + w), Mm(PAGE_H - y)), false),
            ],
            is_closed: false,
        });
    }

    fn circle(&self, cx: f32, cy: f32, r: f32, c: (f32, f32, f32)) {
        self.fill(c);
        let pts = calculate_points_for_circle(Mm(r), Mm(cx), Mm(PAGE_H - cy));
        self.layer.add_polygon(Polygon {
            rings: vec![pts],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        });
    }

    /// Split `**bold**` / `*italic*` markup into styled runs.
    fn parse(&self, src: &str, base: S) -> Vec<(String, S)> {
        let mut out = Vec::new();
        let mut buf = String::new();
        let mut cur = base;
        let ch: Vec<char> = src.chars().collect();
        let mut i = 0;
        while i < ch.len() {
            let bold = ch[i] == '*' && i + 1 < ch.len() && ch[i + 1] == '*';
            let ital = ch[i] == '*' && !bold;
            if bold || ital {
                if !buf.is_empty() {
                    out.push((std::mem::take(&mut buf), cur));
                }
                let want = if bold { S::Bold } else { S::Ital };
                cur = if cur == want { base } else { want };
                i += if bold { 2 } else { 1 };
            } else {
                buf.push(ch[i]);
                i += 1;
            }
        }
        if !buf.is_empty() {
            out.push((buf, cur));
        }
        out
    }

    fn mk_word(&self, segs: Vec<Seg>, size: f32) -> Word {
        let w = segs.iter().map(|s| self.width(&s.text, s.style, size)).sum();
        Word { segs, w }
    }

    fn words(&self, src: &str, base: S, size: f32) -> Vec<Word> {
        let mut out = Vec::new();
        let mut cur: Vec<Seg> = Vec::new();
        for (run, style) in self.parse(src, base) {
            for ch in run.chars() {
                if ch.is_whitespace() {
                    if !cur.is_empty() {
                        out.push(self.mk_word(std::mem::take(&mut cur), size));
                    }
                    continue;
                }
                match cur.last_mut() {
                    Some(last) if last.style == style => last.text.push(ch),
                    _ => cur.push(Seg {
                        text: ch.to_string(),
                        style,
                    }),
                }
            }
        }
        if !cur.is_empty() {
            out.push(self.mk_word(cur, size));
        }
        out
    }

    /// Number of lines a paragraph will occupy at the given width.
    fn lines(&self, src: &str, max_w: f32, size: f32, base: S) -> usize {
        let space = self.width(" ", base, size);
        let words = self.words(src, base, size);
        let mut n = 1;
        let mut lw = 0.0f32;
        for w in &words {
            let add = if lw == 0.0 { w.w } else { space + w.w };
            if lw > 0.0 && lw + add > max_w {
                n += 1;
                lw = w.w;
            } else {
                lw += add;
            }
        }
        n
    }

    /// Ragged-right paragraph. Returns the baseline y of the last line drawn.
    #[allow(clippy::too_many_arguments)]
    fn para(
        &self,
        src: &str,
        x: f32,
        y: f32,
        max_w: f32,
        size: f32,
        lead: f32,
        base: S,
        c: (f32, f32, f32),
    ) -> f32 {
        let space = self.width(" ", base, size);
        let words = self.words(src, base, size);
        let mut line: Vec<&Word> = Vec::new();
        let mut line_w = 0.0f32;
        let mut cy = y;

        let flush = |line: &Vec<&Word>, cy: f32| {
            let mut cx = x;
            for w in line {
                for seg in &w.segs {
                    self.text(&seg.text, cx, cy, size, seg.style, c);
                    cx += self.width(&seg.text, seg.style, size);
                }
                cx += space;
            }
        };

        for w in &words {
            let add = if line.is_empty() { w.w } else { space + w.w };
            if !line.is_empty() && line_w + add > max_w {
                flush(&line, cy);
                line.clear();
                cy += lead;
                line.push(w);
                line_w = w.w;
            } else {
                line.push(w);
                line_w += add;
            }
        }
        if !line.is_empty() {
            flush(&line, cy);
        }
        cy
    }

    /// Register a clickable URI annotation over a text line.
    /// `y` is the text baseline measured from the page top; the hit box is grown
    /// to roughly the ascender/descender so the whole glyph run is clickable.
    fn link(&self, x: f32, y: f32, w: f32, size: f32, url: &str) {
        let asc = size * PT * 0.85;
        let desc = size * PT * 0.28;
        self.layer.add_link_annotation(LinkAnnotation::new(
            Rect::new(
                Mm(x),
                Mm(PAGE_H - y - desc),
                Mm(x + w),
                Mm(PAGE_H - y + asc),
            ),
            // [0 0 0] = no visible border drawn by the viewer.
            Some(BorderArray::Solid([0.0, 0.0, 0.0])),
            Some(ColorArray::Transparent),
            Actions::uri(url.to_string()),
            Some(HighlightingMode::Invert),
        ));
    }

    /// Draw a string and make it clickable. Returns its width.
    fn text_link(&self, t: &str, x: f32, y: f32, size: f32, s: S, c: (f32, f32, f32), url: &str) -> f32 {
        let w = self.width(t, s, size);
        self.text(t, x, y, size, s, c);
        self.link(x, y, w, size, url);
        w
    }

    /// Horizontally centred single line.
    fn text_center(&self, t: &str, cx: f32, y: f32, size: f32, s: S, c: (f32, f32, f32)) {
        let w = self.width(t, s, size);
        self.text(t, cx - w / 2.0, y, size, s, c);
    }

    /// Letter-spaced kicker.
    fn tracked(&self, t: &str, x: f32, y: f32, size: f32, track: f32, s: S, c: (f32, f32, f32)) {
        let mut cx = x;
        for ch in t.chars() {
            let g = ch.to_string();
            self.text(&g, cx, y, size, s, c);
            cx += self.width(&g, s, size) + track;
        }
    }
}

fn face(path: &str) -> ttf_parser::Face<'static> {
    let data: &'static [u8] = Box::leak(std::fs::read(path).unwrap().into_boxed_slice());
    ttf_parser::Face::parse(data, 0).unwrap()
}

fn main() {
    let (pdf, page, layer) = PdfDocument::new(
        "Educational Engineering – Gruppe für Erziehungspersonen",
        Mm(PAGE_W),
        Mm(PAGE_H),
        "Inhalt",
    );

    let dir = "/usr/share/fonts/dejavu";
    let files = [
        "DejaVuSans.ttf",
        "DejaVuSans-Bold.ttf",
        "DejaVuSans-Oblique.ttf",
    ];
    let refs: Vec<IndirectFontRef> = files
        .iter()
        .map(|f| {
            pdf.add_external_font(File::open(format!("{dir}/{f}")).unwrap())
                .unwrap()
        })
        .collect();
    let faces: Vec<ttf_parser::Face<'static>> =
        files.iter().map(|f| face(&format!("{dir}/{f}"))).collect();

    let d = Doc {
        layer: pdf.get_page(page).get_layer(layer),
        refs,
        faces,
    };

    // ---------------- Header ----------------
    let mut y = 20.0;
    d.tracked(
        "GRUPPE FÜR ELTERN UND LEHRPERSONEN",
        M_L,
        y,
        7.6,
        0.55,
        S::Bold,
        GREEN,
    );

    y += 11.5;
    d.text("Educational Engineering", M_L, y, 25.0, S::Bold, GREEN);
    y += 9.8;
    d.text("Erziehung ist Konstruktionsarbeit.", M_L, y, 25.0, S::Bold, INK);

    y += 7.6;
    d.para(
        "Werkzeuge statt Ratschläge – für den Alltag mit AD(H)S-Kindern und Jugendlichen.",
        M_L,
        y,
        180.0,
        11.5,
        5.6,
        S::Reg,
        GREY,
    );

    y += 4.2;
    d.hline(M_L, y, 180.0, 2.5, GREEN);

    // ---------------- Lead ----------------
    y += 6.2;
    y = d.para(
        "Ein Ingenieur, dem eine Brücke schwingt, sucht keinen Schuldigen. Er sucht die Kräfte, \
         die auf sie wirken. **Educational Engineering** überträgt diese Haltung auf die Erziehung: \
         Wir fragen nicht, wer versagt hat – wir fragen, wie das System aus Kind, Familie und Schule \
         konstruiert ist und an welcher Stelle eine kleine, präzise Änderung die grösste Wirkung hat.",
        M_L,
        y,
        180.0,
        10.4,
        5.0,
        S::Reg,
        INK,
    );

    // ---------------- Columns ----------------
    let col_top = y + 9.0;
    let lx = M_L;
    let lw = 102.0;
    let rx = M_L + lw + 7.0;
    let rw = 71.0;

    let body = 9.6f32;
    let lead = 4.85f32;

    // -- left column --
    let mut y = col_top;
    d.text("Warum gerade bei AD(H)S?", lx, y, 10.5, S::Bold, GREEN);
    y += 5.2;
    y = d.para(
        "Kinder und Jugendliche mit AD(H)S haben ein spezielles Temperament mit besonderen \
         Veranlagungen und Talenten. Sie sind keine fehlerhafte Konstruktion – sie sind eine \
         **Hochleistungsmaschine mit anderer Bauart**: schnell, sprunghaft, kreativ, reizoffen. \
         Mit Standardmethoden gefahren, überhitzt sie. Richtig verstanden und richtig eingestellt, \
         läuft sie zu einer Form auf, die dem erzieherischen Umfeld sonst verborgen bleibt.",
        lx,
        y,
        lw,
        body,
        lead,
        S::Reg,
        INK,
    );
    y += lead + 1.4;
    y = d.para(
        "Genau darin liegt die grosse Herausforderung für das erzieherische Umfeld – und genau \
         dort setzt diese Gruppe an.",
        lx,
        y,
        lw,
        body,
        lead,
        S::Reg,
        INK,
    );

    y += 8.4;
    d.text("Die Arbeitsweise", lx, y, 10.5, S::Bold, GREEN);
    y += 5.4;

    let steps = [
        "**Analysieren statt bewerten.** Wir zerlegen die Situation in ihre Bestandteile, bevor \
         wir sie beurteilen.",
        "**Das System sehen.** Verhalten entsteht zwischen Menschen, nicht in einem Kind allein. \
         Wer die Umgebung ändert, ändert das Verhalten.",
        "**Strategien konstruieren.** Gemeinsam mit der Fachperson, Dr. med. Ursula Davatz, \
         erarbeiten wir konkrete Problemlösungsstrategien – zugeschnitten auf Ihre reale Situation.",
        "**Testen und nachjustieren.** Sie setzen die Strategie im Alltag um und bringen die \
         Erfahrung zurück in die Gruppe. Jede Sitzung ist eine neue Iteration.",
    ];

    for (i, s) in steps.iter().enumerate() {
        let r = 2.75;
        let cy = y - 1.15;
        d.circle(lx + r, cy, r, GREEN);
        let n = (i + 1).to_string();
        let nw = d.width(&n, S::Bold, 7.6);
        d.text(&n, lx + r - nw / 2.0, cy + 1.0, 7.6, S::Bold, (1.0, 1.0, 1.0));
        y = d.para(s, lx + 8.6, y, lw - 8.6, body, lead, S::Reg, INK);
        y += lead + 1.2;
    }

    y += 3.6;
    d.text("Was Sie mitnehmen", lx, y, 10.5, S::Bold, GREEN);
    y += 5.2;
    let left_end = d.para(
        "Die Erfahrung von Erziehungspersonen, die dieselben Kräfte kennen – und Strategien, die \
         nicht in der Theorie bleiben, sondern am nächsten Morgen am Küchentisch oder im \
         Schulzimmer funktionieren müssen.",
        lx,
        y,
        lw,
        body,
        lead,
        S::Reg,
        INK,
    );

    // -- right column: facts box --
    let pad = 4.0;
    let bx = rx;
    let box_top = col_top - 5.6;
    let tx = bx + pad;
    let dt_w = 16.0;
    let dd_x = tx + dt_w + 2.0;
    let dd_w = bx + rw - pad - dd_x;
    let fsize = 9.0f32;
    let flead = 4.5f32;

    let rows: [(&str, &str); 6] = [
        ("Zeit", "Jeweils Donnerstag\n17.30 – 19.00 Uhr"),
        ("Beginn", "29.01.2026"),
        (
            "Daten",
            "29.01. · 26.02. · 26.03.\n30.04. · 21.05. · 25.06.\n27.08. · 24.09. · 29.10.\n26.11. · 17.12.2026",
        ),
        (
            "Ort",
            "Praxis „Zum grünen Haus“\nWinterthurerstrasse 52\n8006 Zürich\n*ab HB Tram Nr. 10 bis Kinkelstrasse*",
        ),
        ("Leitung", "Dr. med. Ursula Davatz"),
        (
            "Kosten",
            "400 Franken, aufgeteilt auf die Anzahl Teilnehmer. Die Krankenkasse übernimmt die Kosten – ausser dem Selbstbehalt.",
        ),
    ];

    // Pre-compute the box height so the background can be drawn first.
    let mut probe = box_top + pad + 3.4 + 5.4;
    for (_, dd) in &rows {
        for l in dd.split('\n') {
            probe += d.lines(l, dd_w, fsize, S::Reg) as f32 * flead;
        }
        probe += 1.6;
    }
    let box_h = probe - box_top + 2.0;

    d.rect(bx, box_top, rw, box_h, GREEN_BG);
    d.rect(bx, box_top, 0.9, box_h, GREEN);

    let mut fy = box_top + pad + 3.4;
    d.text("Die Fakten", tx, fy, 10.5, S::Bold, GREEN);
    fy += 5.4;

    for (dt, dd) in &rows {
        d.text(dt, tx, fy, fsize, S::Bold, GREEN);
        let mut ly = fy;
        for l in dd.split('\n') {
            ly = d.para(l, dd_x, ly, dd_w, fsize, flead, S::Reg, INK);
            ly += flead;
        }
        fy = ly + 1.6;
    }

    // ---------------- Call to action ----------------
    // Sit below whichever column runs longer, rather than at a guessed offset.
    let band_h = 28.0;
    let band_y = left_end.max(box_top + box_h) + 7.5;
    let cx = PAGE_W / 2.0;
    d.rect(M_L, band_y, 180.0, band_h, GREEN);

    d.text_center(
        "Keine Anmeldung. Kein Formular. Sie kommen einfach vorbei.",
        cx,
        band_y + 9.8,
        13.5,
        S::Bold,
        (1.0, 1.0, 1.0),
    );
    d.text_center(
        "Erster Termin: Donnerstag, 29. Januar 2026, 17.30 – 19.00 Uhr",
        cx,
        band_y + 17.4,
        10.4,
        S::Reg,
        (1.0, 1.0, 1.0),
    );
    d.text_center(
        "Praxis „Zum grünen Haus“ · Winterthurerstrasse 52 · 8006 Zürich · ab HB Tram Nr. 10 bis Kinkelstrasse",
        cx,
        band_y + 23.6,
        8.8,
        S::Reg,
        (0.82, 0.90, 0.84),
    );

    // ---------------- Footer ----------------
    let foot = PAGE_H - 12.0;
    d.hline(M_L, foot - 3.0, 180.0, 0.8, RULE);
    d.text(
        "Leitung: Dr. med. Ursula Davatz · Praxis „Zum grünen Haus“, Winterthurerstrasse 52, 8006 Zürich",
        M_L,
        foot,
        8.0,
        S::Reg,
        GREY,
    );
    // Right-aligned, clickable. Drawn in green so they read as links.
    let sep = " · ";
    let sites = [
        ("ganglion.ch", "https://ganglion.ch"),
        ("adhs.expert", "https://adhs.expert"),
    ];
    let sep_w = d.width(sep, S::Reg, 8.0);
    let total: f32 = sites
        .iter()
        .map(|(t, _)| d.width(t, S::Reg, 8.0))
        .sum::<f32>()
        + sep_w * (sites.len() - 1) as f32;

    let mut fx = PAGE_W - M_L - total;
    for (i, (label, url)) in sites.iter().enumerate() {
        if i > 0 {
            d.text(sep, fx, foot, 8.0, S::Reg, GREY);
            fx += sep_w;
        }
        fx += d.text_link(label, fx, foot, 8.0, S::Reg, GREEN, url);
    }

    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "educational_engineering.pdf".to_string());
    pdf.save(&mut BufWriter::new(File::create(&out).unwrap()))
        .unwrap();
    println!("wrote {out}");
}
