// Generates a one-page A4 flyer (German) for Dr. Davatz's ADHS/ADS teacher-training course.
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
const WHITE: (f32, f32, f32) = (1.0, 1.0, 1.0);
const PALE: (f32, f32, f32) = (0.82, 0.90, 0.84);

/// Map link for the practice address. Percent-encoded so the PDF /URI string stays ASCII.
const MAP_URL: &str =
    "https://www.google.com/maps/search/?api=1&query=Winterthurerstrasse+52%2C+8006+Z%C3%BCrich";
const WEB_URL: &str = "https://www.ganglion.ch";
const MAIL_URL: &str = "mailto:sekretariat@ganglion.ch";
const TEL_URL: &str = "tel:+41582550115";

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
        "Weiterbildungskurs im Umgang mit ADHS- und ADS-Kindern und Jugendlichen",
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
    let mut y = 16.5;
    d.tracked(
        "PSYCHIATRISCHE PRAXIS DR. MED. URSULA DAVATZ",
        M_L,
        y,
        7.2,
        0.55,
        S::Bold,
        GREEN,
    );
    y += 5.2;
    d.tracked(
        "KOMPETENZZENTRUM FÜR AD(H)S & FOLGEKRANKHEITEN",
        M_L,
        y,
        7.2,
        0.55,
        S::Bold,
        GREY,
    );

    y += 13.5;
    d.text("Weiterbildungskurs", M_L, y, 27.0, S::Bold, GREEN);
    y += 10.6;
    y = d.para(
        "im Umgang mit ADHS- und ADS-Kindern und Jugendlichen",
        M_L,
        y,
        180.0,
        16.0,
        7.6,
        S::Bold,
        INK,
    );

    y += 6.4;
    d.text("Educational Engineering", M_L, y, 11.0, S::Ital, GREEN);
    d.text(
        " – Erziehung ist Konstruktionsarbeit.",
        M_L + d.width("Educational Engineering", S::Ital, 11.0),
        y,
        11.0,
        S::Ital,
        GREY,
    );

    y += 4.0;
    d.hline(M_L, y, 180.0, 2.5, GREEN);

    // ---------------- Lead (course goal) ----------------
    y += 7.0;
    y = d.para(
        "**Kursziel.** Sie lernen den kompetenten Umgang mit neurodiversen Kindern und \
         Jugendlichen, damit eine Folgestörung möglichst verhindert werden kann. Statt Schuldige \
         zu suchen, verstehen wir die Kräfte im System aus Kind, Familie und Schule, ordnen sie \
         ein und vermitteln zwischen den unterschiedlichen Gesichtspunkten.",
        M_L,
        y,
        180.0,
        11.5,
        5.6,
        S::Reg,
        INK,
    );

    // ---------------- Columns ----------------
    let col_top = y + 12.0;
    let lx = M_L;
    let lw = 100.0;
    let rx = M_L + lw + 7.0;
    let rw = 73.0;

    let body = 10.2f32;
    let lead = 5.45f32;

    // -- left column: course flow + themes --
    let mut y = col_top;
    d.text("Kursablauf", lx, y, 11.0, S::Bold, GREEN);
    y += 5.8;
    y = d.para(
        "Zu Beginn jedes Kurstages gibt es einen theoretischen Input der Kursleiterin, gefolgt von \
         der praktischen Anwendung anhand von Fallbeispielen und Diskussionen über Lösungsansätze. \
         Die Themen umfassen:",
        lx,
        y,
        lw,
        body,
        lead,
        S::Reg,
        INK,
    );
    y += lead + 3.8;

    let themes = [
        "Einführung in ADHS, ADS und ASS sowie persönliche Erziehungserfahrungen.",
        "Konflikte im Umgang mit betroffenen Kindern und Lösungsstrategien (Do’s and Don’ts).",
        "Gruppendynamik und Konfliktlösung im Klassenzimmer und auf dem Pausenplatz.",
        "Umgang mit Eltern von ADHS-, ADS- und ASS-Kindern.",
        "Herausforderungen der integrativen Schule mit betroffenen Kindern.",
        "Vorbeugung von Folgekrankheiten durch angemessenen Umgang mit neurodiversen Kindern.",
    ];
    for t in themes.iter() {
        d.circle(lx + 1.2, y - 1.15, 1.2, GREEN);
        y = d.para(t, lx + 6.2, y, lw - 6.2, body, lead, S::Reg, INK);
        y += lead + 3.0;
    }
    let left_end = y - 3.0;

    // -- right column: facts box --
    let pad = 4.5;
    let bx = rx;
    let box_top = col_top - 6.0;
    let tx = bx + pad;
    let dt_w = 16.0;
    let dd_x = tx + dt_w + 2.0;
    let dd_w = bx + rw - pad - dd_x;
    let fsize = 8.8f32;
    let flead = 4.8f32;

    // (label, [(line, is_part_of_the_address)]) — address lines get the map link.
    let rows: [(&str, &[(&str, bool)]); 5] = [
        (
            "Für wen",
            &[(
                "Staatliche Erziehungspersonen wie Lehrer/innen, Kindergärtner/innen und Hortleiter/innen.",
                false,
            )],
        ),
        (
            "Leitung",
            &[(
                "Frau Dr. med. Ursula Davatz, Fachärztin FMH für Psychiatrie und Psychotherapie, Familientherapeutin nach Murray Bowen.",
                false,
            )],
        ),
        (
            "Daten",
            &[
                ("26.08. · 23.09. · 21.10.2026", false),
                ("25.11.2026", false),
                ("24.03. · 19.05.2027", false),
                ("*Jeweils 14.00 – 18.00 Uhr*", false),
            ],
        ),
        (
            "Ort",
            &[
                ("Psychiatrische Praxis Dr. med. Ursula Davatz", true),
                ("Winterthurerstrasse 52, 8006 Zürich", true),
            ],
        ),
        (
            "Kosten",
            &[(
                "CHF 1’200.00 pro Person (6 Daten). Durchführung ab 6 Personen, max. 12 Teilnehmer.",
                false,
            )],
        ),
    ];

    // Pre-compute the box height so the background can be drawn first.
    let mut probe = box_top + pad + 3.4 + 5.6;
    for (_, lines) in &rows {
        for (l, _) in lines.iter() {
            probe += d.lines(l, dd_w, fsize, S::Reg) as f32 * flead;
        }
        probe += 2.6;
    }
    let box_h = probe - box_top + 2.0;

    d.rect(bx, box_top, rw, box_h, GREEN_BG);
    d.rect(bx, box_top, 0.9, box_h, GREEN);

    let mut fy = box_top + pad + 3.4;
    d.text("Auf einen Blick", tx, fy, 11.0, S::Bold, GREEN);
    fy += 5.6;

    for (dt, lines) in &rows {
        d.text(dt, tx, fy, fsize, S::Bold, GREEN);
        let mut ly = fy;
        for (l, is_addr) in lines.iter() {
            // Address lines are green + clickable, matching the URLs in the footer.
            let col = if *is_addr { GREEN } else { INK };
            if *is_addr {
                d.link(dd_x, ly, d.width(l, S::Reg, fsize), fsize, MAP_URL);
            }
            ly = d.para(l, dd_x, ly, dd_w, fsize, flead, S::Reg, col);
            ly += flead;
        }
        fy = ly + 2.6;
    }

    // ---------------- Call to action ----------------
    // Sit below whichever column runs longer, rather than at a guessed offset.
    let band_h = 31.0;
    let band_y = left_end.max(box_top + box_h) + 10.5;
    let cx = PAGE_W / 2.0;
    d.rect(M_L, band_y, 180.0, band_h, GREEN);

    d.text_center("Anmeldung", cx, band_y + 10.4, 14.0, S::Bold, WHITE);
    // Clickable e-mail, centred. White so it stays visible on the green band.
    let mail = "sekretariat@ganglion.ch";
    let mw = d.width(mail, S::Bold, 13.0);
    d.text_link(mail, cx - mw / 2.0, band_y + 19.2, 13.0, S::Bold, WHITE, MAIL_URL);
    d.text_center(
        "Der Kurs findet ab sechs Personen statt · höchstens zwölf Teilnehmer",
        cx,
        band_y + 26.0,
        8.6,
        S::Reg,
        PALE,
    );

    // ---------------- Footer ----------------
    let foot_rule = PAGE_H - 19.0;
    d.hline(M_L, foot_rule, 180.0, 0.8, RULE);
    d.text_center(
        "Psychiatrische Praxis Dr. med. Ursula Davatz · Kompetenzzentrum für AD(H)S & Folgekrankheiten",
        cx,
        foot_rule + 5.4,
        7.6,
        S::Reg,
        GREY,
    );
    // Contact line: each segment is a clickable link (map, phone, mail, web).
    let contact: [(&str, &str); 4] = [
        ("Winterthurerstrasse 52, 8006 Zürich", MAP_URL),
        ("Tel. 058 255 01 15", TEL_URL),
        ("sekretariat@ganglion.ch", MAIL_URL),
        ("www.ganglion.ch", WEB_URL),
    ];
    let csep = "   ·   ";
    let csize = 8.2f32;
    let csep_w = d.width(csep, S::Reg, csize);
    let ctotal: f32 = contact
        .iter()
        .map(|(t, _)| d.width(t, S::Bold, csize))
        .sum::<f32>()
        + csep_w * (contact.len() - 1) as f32;
    let mut fx = cx - ctotal / 2.0;
    let cfy = foot_rule + 11.2;
    for (i, (label, url)) in contact.iter().enumerate() {
        if i > 0 {
            d.text(csep, fx, cfy, csize, S::Reg, GREY);
            fx += csep_w;
        }
        fx += d.text_link(label, fx, cfy, csize, S::Bold, GREEN, url);
    }

    // Defaults to the _2 file so a plain `cargo run` regenerates the course flyer
    // without clobbering the retained old parenting-group flyer (educational_engineering.pdf).
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "educational_engineering_2.pdf".to_string());
    pdf.save(&mut BufWriter::new(File::create(&out).unwrap()))
        .unwrap();
    println!("wrote {out}");
}
