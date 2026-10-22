//! Local reconstruction: REA parity.md names formats, not a recovered generator.
//! Small, deterministic writers reuse the bounded ZIP dependency already used by
//! the reader. No macros, formulas, remote assets, scripts or shell execution.
use crate::documents;
use crate::pdf_font;
use grokbot_engine::contracts::{Error, Result};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Write};
use unicode_normalization::UnicodeNormalization;
use zip::{write::SimpleFileOptions, ZipWriter};

const MAX_CONTENT: usize = 100_000;
const MAX_LINES: usize = 1800;
const MAX_TABLE_COLS: usize = 50;
const MAX_TABLE_ROWS: usize = 1000;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub name: String,
    pub format: String,
    pub content: String,
}
pub struct Generated {
    pub name: String,
    pub format: String,
    pub bytes: Vec<u8>,
    pub preview: String,
    pub pages: usize,
}
fn invalid(s: &str) -> Error {
    Error::InvalidInput(s.into())
}
pub fn mime(format: &str) -> &'static str {
    match format {
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "csv" => "text/csv; charset=utf-8",
        "svg" => "image/svg+xml",
        _ => "text/markdown; charset=utf-8",
    }
}
fn valid_text(s: &str) -> bool {
    s.chars().all(|c| {
        (!c.is_control() || matches!(c, '\n' | '\r' | '\t'))
            && !matches!(c, '\u{fffe}' | '\u{ffff}')
    })
}
pub fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
// Keeps runs of spaces and indentation (tab = 4 spaces): aligned columns stay
// aligned in Courier. Breaks on the last space that fits, else mid-word.
fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for line in s.lines() {
        let mut rest: Vec<char> = line.replace('\t', "    ").trim_end().chars().collect();
        while rest.len() > width {
            let indent = rest.iter().take_while(|c| **c == ' ').count();
            match (indent + 1..=width).rev().find(|&i| rest[i] == ' ') {
                Some(i) => {
                    lines.push(rest[..i].iter().collect::<String>().trim_end().to_string());
                    rest.drain(..=i);
                }
                None => {
                    lines.push(rest[..width].iter().collect());
                    rest.drain(..width);
                }
            }
        }
        lines.push(rest.into_iter().collect());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}
pub fn generate(request: &Request) -> Result<Generated> {
    if request.name.trim().is_empty()
        || request.name.len() > 180
        || request.name.contains(['/', '\\'])
        || request.name.starts_with('.')
        || request.name.chars().any(char::is_control)
    {
        return Err(invalid(
            "Le nom doit être un simple nom de fichier, sans dossier (180 octets maximum).",
        ));
    }
    if request.content.trim().is_empty()
        || request.content.len() > MAX_CONTENT
        || !valid_text(&request.content)
    {
        return Err(invalid(
            "Le contenu est vide, invalide ou dépasse la limite de 100 000 octets.",
        ));
    }
    let format = request.format.as_str();
    if !["pdf", "docx", "pptx", "md", "csv", "xlsx", "svg"].contains(&format) {
        return Err(invalid(
            "Format de génération non accepté : PDF, DOCX, PPTX, Markdown, CSV, XLSX ou SVG.",
        ));
    }
    let name = if request.name.to_lowercase().ends_with(&format!(".{format}")) {
        request.name.clone()
    } else {
        format!("{}.{format}", request.name)
    };
    let normalized = request.content.nfc().collect::<String>();
    let lines = wrap(&normalized, 70);
    if lines.len() > MAX_LINES {
        return Err(invalid(
            "Le document dépasse la limite de mise en page (1 800 lignes).",
        ));
    }
    let mut preview_lines = lines.clone();
    let mut table = None;
    let mut pages = lines.len().div_ceil(48);
    let mut rich = None;
    let bytes = match format {
        "md" => request.content.as_bytes().to_vec(),
        "pdf" => {
            let (bytes, n) = pdf(&normalized)?;
            pages = n;
            if document_rich(&normalized)? {
                rich = Some(blocks_preview("pdf", &blocks(&normalized)?));
            }
            bytes
        }
        "docx" => {
            if document_rich(&normalized)? {
                rich = Some(blocks_preview("docx", &blocks(&normalized)?));
            }
            docx(&normalized)?
        }
        "pptx" => {
            let deck = slides(&request.content)?;
            pages = deck.len();
            preview_lines = deck[0].lines();
            if deck[0].rich() {
                rich = Some(blocks_preview("pptx", &deck[0].as_blocks()));
            }
            pptx(&deck)?
        }
        "csv" | "xlsx" => {
            let rows: Vec<Vec<String>> = serde_json::from_str(&request.content).map_err(|_| invalid("Pour CSV ou Excel, content doit être un tableau JSON de lignes de chaînes : [[\"Nom\",\"Valeur\"],[\"Été\",\"12\"]]."))?;
            if rows.is_empty()
                || rows.len() > 1000
                || rows.iter().any(|r| {
                    r.is_empty()
                        || r.len() > 50
                        || r.iter().any(|s| s.chars().count() > 1000 || !valid_text(s))
                })
            {
                return Err(invalid("Le tableau doit contenir 1 à 1 000 lignes, 1 à 50 colonnes et au plus 1 000 caractères par cellule."));
            }
            table = Some(table_preview(&rows));
            pages = 1;
            if format == "csv" {
                csv(&rows)
            } else {
                xlsx(&rows)?
            }
        }
        "svg" => {
            let svg = diagram(&request.content)?;
            return Ok(Generated {
                name,
                format: format.into(),
                bytes: svg.as_bytes().to_vec(),
                preview: svg,
                pages: 1,
            });
        }
        _ => unreachable!(),
    };
    if bytes.len() > documents::MAX_BYTES {
        return Err(invalid("Le fichier généré dépasse 20 Mo."));
    }
    let preview =
        rich.unwrap_or_else(|| table.unwrap_or_else(|| text_preview(&preview_lines, format)));
    Ok(Generated {
        name,
        format: format.into(),
        bytes,
        preview,
        pages,
    })
}
/// A spreadsheet has cells, not pages. Keep a bounded, readable thumbnail even
/// for a three-row table; truncate only the preview, never the stored data.
fn table_preview(rows: &[Vec<String>]) -> String {
    let cols = rows.iter().map(Vec::len).max().unwrap_or(1).min(6);
    let count = rows.len().min(8);
    let w = 640;
    let h = count * 32 + 24;
    let cell = 616.0 / cols as f64;
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\" font-family=\"Arial,Helvetica,sans-serif\" font-size=\"14\"><rect width=\"{w}\" height=\"{h}\" fill=\"#fff\"/>");
    for (r, row) in rows.iter().take(count).enumerate() {
        for c in 0..cols {
            let (x, y) = (12.0 + c as f64 * cell, 12 + r * 32);
            let text = row
                .get(c)
                .map(String::as_str)
                .unwrap_or("")
                .replace(['\n', '\r', '\t'], " ");
            let limit = ((cell - 16.0) / 8.0) as usize;
            let mut shown: String = text.chars().take(limit).collect();
            if text.chars().count() > limit {
                shown.pop();
                shown.push('…');
            }
            svg.push_str(&format!("<rect x=\"{x:.1}\" y=\"{y}\" width=\"{cell:.1}\" height=\"32\" fill=\"{}\" stroke=\"#e2e8f0\"/><text x=\"{:.1}\" y=\"{}\" fill=\"#172033\"{}>{}</text>", if r == 0 { "#f1f5f9" } else { "#fff" }, x + 8.0, y + 21, if r == 0 { " font-weight=\"bold\"" } else { "" }, xml(&shown)));
        }
    }
    svg.push_str("</svg>");
    svg
}
fn text_preview(lines: &[String], format: &str) -> String {
    let slide = format == "pptx";
    let cropped = matches!(format, "md" | "docx");
    let (w, h, count) = if slide {
        (960, 540, 16)
    } else {
        (595, 842, 48)
    };
    let reflowed;
    let lines = if cropped {
        reflowed = wrap(&lines.join("\n"), 47);
        reflowed.as_slice()
    } else {
        lines
    };
    // Thumbnails use the file's own family. PDF embeds DejaVu Sans; an emoji
    // font is only a fallback so 🎉 is visible in the card.
    let family = if format == "pdf" {
        "DejaVu Sans,Noto Color Emoji,Apple Color Emoji,Segoe UI Emoji,sans-serif"
    } else if matches!(format, "docx" | "pptx") {
        "Arial,Helvetica,Noto Color Emoji,sans-serif"
    } else {
        "Courier New,Courier,monospace"
    };
    let mut body = format!(
        "<g fill=\"#172033\" font-family=\"{family}\" font-size=\"12\" xml:space=\"preserve\">"
    );
    let mut y = 42;
    let mut right: f64 = 320.0;
    for (i, line) in lines.iter().take(count).enumerate() {
        let (heading, text) = if matches!(format, "docx" | "md") {
            heading(line)
        } else {
            (0, line.as_str())
        };
        let (font_size, advance) = match (slide, heading) {
            (true, _) if i == 0 => (28, 28),
            (true, _) => (18, 28),
            (false, 1) => (18, 26),
            (false, 2) => (15, 22),
            (false, 3) => (16, 22),
            _ if cropped => (16, 22),
            _ => (12, 15),
        };
        y += advance;
        // Clip long table cells explicitly in the thumbnail, not in the file.
        let visible = text
            .chars()
            .take(if slide { 80 } else { 70 })
            .collect::<String>();
        right = right.max(84.0 + visible.chars().count() as f64 * font_size as f64 * 0.6);
        body.push_str(&format!(
            "<text x=\"42\" y=\"{y}\" font-size=\"{font_size}\"{}>{}</text>",
            if heading > 0 {
                " font-weight=\"bold\""
            } else {
                ""
            },
            xml(&visible)
        ));
        if y > h - 40 {
            break;
        }
    }
    let w = if cropped {
        (right.ceil() as i32).min(w)
    } else {
        w
    };
    let h = if cropped {
        (y + 42).max(w * 45 / 100).min(h)
    } else {
        h
    };
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\"><rect width=\"{w}\" height=\"{h}\" fill=\"#fff\"/>{body}</g></svg>")
}
/// Markdown heading level (1 to 3) and its text; 0 for an ordinary line.
fn heading(line: &str) -> (usize, &str) {
    for (level, marker) in [(3, "### "), (2, "## "), (1, "# ")] {
        if let Some(text) = line.strip_prefix(marker) {
            return (level, text.trim());
        }
    }
    (0, line)
}
// WinAnsi repertoire already accepted for French, plus the party emoji drawn
// as an image. Anything else fails in French: a missing glyph is never painted.
fn unsupported_pdf(c: char) -> Error {
    invalid(&format!(
        "Ce PDF contient « {c} » (U+{:04X}), hors de la police embarquée. Aucun fichier n’est produit.",
        c as u32
    ))
}
fn pdf_byte(c: char) -> Result<u8> {
    let Some(b) = pdf_font::encode(c) else {
        return Err(unsupported_pdf(c));
    };
    if pdf_font::face().glyphs[b as usize] == 0 {
        return Err(unsupported_pdf(c));
    }
    Ok(b)
}
enum Block {
    Text(String),
    Table(Vec<Vec<String>>),
}
enum Item {
    Line(String),
    Table(Vec<Vec<String>>),
}
struct Slide {
    items: Vec<Item>,
}
impl Slide {
    fn lines(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|item| match item {
                Item::Line(line) => Some(line.clone()),
                Item::Table(_) => None,
            })
            .collect()
    }
    fn rich(&self) -> bool {
        self.items.iter().any(|item| match item {
            Item::Table(_) => true,
            Item::Line(line) => line.contains(pdf_font::PARTY),
        })
    }
    fn as_blocks(&self) -> Vec<Block> {
        self.items
            .iter()
            .map(|item| match item {
                Item::Line(line) => Block::Text(line.clone()),
                Item::Table(rows) => Block::Table(rows.clone()),
            })
            .collect()
    }
}
fn document_rich(content: &str) -> Result<bool> {
    Ok(content.contains(pdf_font::PARTY)
        || blocks(content)?
            .iter()
            .any(|b| matches!(b, Block::Table(_))))
}
fn row_cells(line: &str) -> Option<Vec<String>> {
    let t = line.trim();
    if !t.contains('|') {
        return None;
    }
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    // A tab is four spaces, same as `wrap`, before any glyph is measured.
    Some(
        t.split('|')
            .map(|c| c.trim().replace('\t', "    "))
            .collect(),
    )
}
fn column_count(line: &str) -> usize {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').count()
}
fn is_separator_line(line: &str) -> bool {
    let t = line.trim();
    if !t.contains('|') {
        return false;
    }
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut any = false;
    for cell in t.split('|') {
        let cell = cell.trim().trim_matches(':');
        if cell.len() < 3 || !cell.bytes().all(|byte| byte == b'-') {
            return false;
        }
        any = true;
    }
    any
}
fn table_span(lines: &[&str], i: usize) -> Option<usize> {
    if !lines[i].contains('|') || is_separator_line(lines[i]) {
        return None;
    }
    let separator = lines.get(i + 1)?;
    if !is_separator_line(separator) {
        return None;
    }
    let mut n = 2;
    while let Some(line) = lines.get(i + n) {
        if !line.contains('|') || is_separator_line(line) {
            break;
        }
        n += 1;
    }
    Some(n)
}
fn table_too_big() -> Error {
    invalid("Le tableau doit contenir 1 à 1 000 lignes et 1 à 50 colonnes.")
}
fn blocks(content: &str) -> Result<Vec<Block>> {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < lines.len() {
        if let Some(n) = table_span(&lines, i) {
            // Refuse before `row_cells` or `resize` reserves a String per cell.
            if n - 1 > MAX_TABLE_ROWS {
                return Err(table_too_big());
            }
            for line in lines.iter().take(i + n).skip(i) {
                if column_count(line) > MAX_TABLE_COLS {
                    return Err(table_too_big());
                }
            }
            let mut rows = vec![row_cells(lines[i]).unwrap_or_default()];
            for line in lines.iter().take(i + n).skip(i + 2) {
                rows.push(row_cells(line).unwrap_or_default());
            }
            let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
            for row in &mut rows {
                row.resize(cols, String::new());
            }
            out.push(Block::Table(rows));
            i += n;
        } else {
            out.push(Block::Text(lines[i].to_string()));
            i += 1;
        }
    }
    if out.is_empty() {
        out.push(Block::Text(String::new()));
    }
    Ok(out)
}
fn blocks_preview(format: &str, blocks: &[Block]) -> String {
    let slide = format == "pptx";
    let (w, page_h) = if slide {
        (960, 540)
    } else if format == "pdf" {
        (595, 842)
    } else {
        (640, 480)
    };
    let family = if format == "pdf" {
        "DejaVu Sans,Noto Color Emoji,Apple Color Emoji,Segoe UI Emoji,sans-serif"
    } else {
        "Arial,Helvetica,Noto Color Emoji,sans-serif"
    };
    let mut body = format!(
        "<g fill=\"#172033\" font-family=\"{family}\" font-size=\"12\" xml:space=\"preserve\">"
    );
    let mut y = 28i32;
    for block in blocks {
        if y > page_h - 28 {
            break;
        }
        match block {
            Block::Text(line) => {
                y += 18;
                let mut shown: String = line.chars().take(90).collect();
                if !shown.contains(pdf_font::PARTY) && line.contains(pdf_font::PARTY) {
                    shown.push(pdf_font::PARTY);
                }
                body.push_str(&format!("<text x=\"42\" y=\"{y}\">{}</text>", xml(&shown)));
            }
            Block::Table(rows) => {
                let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let shown_rows = rows.len().min(12);
                let cw = (w - 84) / cols as i32;
                for (r, row) in rows.iter().take(shown_rows).enumerate() {
                    for c in 0..cols {
                        let x = 42 + c as i32 * cw;
                        let fill = if r == 0 { "#f1f5f9" } else { "#ffffff" };
                        let cell = row.get(c).map(String::as_str).unwrap_or("");
                        let text: String = cell.chars().take(40).collect();
                        body.push_str(&format!("<rect x=\"{x}\" y=\"{y}\" width=\"{cw}\" height=\"22\" fill=\"{fill}\" stroke=\"#e2e8f0\"/><text x=\"{}\" y=\"{}\">{}</text>", x + 6, y + 15, xml(&text)));
                    }
                    y += 22;
                    if y > page_h - 28 {
                        break;
                    }
                }
                y += 10;
            }
        }
    }
    let h = if format == "pdf" {
        page_h
    } else {
        (y + 28).clamp(120, page_h)
    };
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\"><rect width=\"{w}\" height=\"{h}\" fill=\"#fff\"/>{body}</g></svg>")
}
const PAGE_TOP: f32 = 786.0;
const PAGE_BOTTOM: f32 = 42.0;
const PAGE_LEFT: f32 = 42.0;
const CONTENT_W: f32 = 511.0;
fn char_width(c: char, size: f32) -> Result<f32> {
    if c == pdf_font::PARTY {
        return Ok(size + 1.5);
    }
    let b = pdf_byte(c)?;
    Ok(pdf_font::face().widths[b as usize] as f32 * size / 1000.0)
}
fn measure(text: &str, size: f32) -> Result<f32> {
    text.chars()
        .try_fold(0.0, |sum, c| Ok(sum + char_width(c, size)?))
}
fn wrap_width(line: &str, max: f32, size: f32) -> Result<Vec<String>> {
    let chars: Vec<char> = line.replace('\t', "    ").chars().collect();
    if chars.is_empty() {
        return Ok(vec![String::new()]);
    }
    let widths: Vec<f32> = chars
        .iter()
        .map(|c| char_width(*c, size))
        .collect::<Result<_>>()?;
    let mut lines = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let mut end = start;
        let mut used = 0.0;
        while end < chars.len() && used + widths[end] <= max {
            used += widths[end];
            end += 1;
        }
        if end == start {
            end = start + 1;
        } else if end < chars.len() {
            if let Some(rel) = chars[start..end].iter().rposition(|c| *c == ' ') {
                if start + rel > start {
                    end = start + rel;
                }
            }
        }
        lines.push(
            chars[start..end]
                .iter()
                .collect::<String>()
                .trim_end()
                .to_string(),
        );
        start = end;
        while start < chars.len() && chars[start] == ' ' {
            start += 1;
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    Ok(lines)
}
fn draw_text(ops: &mut Vec<u8>, text: &str, x: f32, y: f32, size: f32) -> Result<f32> {
    if text.is_empty() {
        return Ok(0.0);
    }
    let face = pdf_font::face();
    let mut encoded = Vec::new();
    let mut width = 0u32;
    for c in text.chars() {
        let b = pdf_byte(c)?;
        width += face.widths[b as usize] as u32;
        if matches!(b, b'(' | b')' | b'\\') {
            encoded.push(b'\\');
        }
        encoded.push(b);
    }
    ops.extend(format!("BT /F1 {size:.2} Tf 1 0 0 1 {x:.2} {y:.2} Tm (").as_bytes());
    ops.extend(encoded);
    ops.extend(b") Tj ET\n");
    Ok(width as f32 * size / 1000.0)
}
fn draw_line(ops: &mut Vec<u8>, line: &str, mut x: f32, baseline: f32, size: f32) -> Result<()> {
    let mut buf = String::new();
    for c in line.chars() {
        if c == pdf_font::PARTY {
            if !buf.is_empty() {
                x += draw_text(ops, &buf, x, baseline, size)?;
                buf.clear();
            }
            let y = baseline - size * 0.2;
            ops.extend(
                format!("q {size:.2} 0 0 {size:.2} {x:.2} {y:.2} cm /Im1 Do Q\n").as_bytes(),
            );
            x += size + 1.5;
        } else {
            buf.push(c);
        }
    }
    if !buf.is_empty() {
        draw_text(ops, &buf, x, baseline, size)?;
    }
    Ok(())
}
struct Painter {
    pages: Vec<Vec<u8>>,
    y: f32,
}
impl Painter {
    fn new() -> Self {
        Self {
            pages: vec![b"0.090 0.125 0.200 rg\n".to_vec()],
            y: PAGE_TOP,
        }
    }
    fn ops(&mut self) -> &mut Vec<u8> {
        self.pages.last_mut().unwrap()
    }
    fn new_page(&mut self) {
        self.pages.push(b"0.090 0.125 0.200 rg\n".to_vec());
        self.y = PAGE_TOP;
    }
    fn ensure(&mut self, height: f32) {
        if self.y - height < PAGE_BOTTOM && self.y < PAGE_TOP - 0.5 {
            self.new_page();
        }
    }
}
/// Lines of a table row that keep every baseline and the cell box on the page.
fn table_lines_that_fit(y: f32) -> usize {
    let mut take = 0usize;
    loop {
        let next = take + 1;
        let row_h = (8.0 + next as f32 * 13.0).max(16.0);
        let baseline = y - 12.0 - take as f32 * 13.0;
        if baseline < PAGE_BOTTOM || y - row_h < PAGE_BOTTOM {
            break;
        }
        take = next;
    }
    take
}
fn paint_table(page: &mut Painter, rows: &[Vec<String>]) -> Result<()> {
    let cols = rows.first().map(Vec::len).unwrap_or(1).max(1);
    let mut pref = vec![48.0f32; cols];
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            pref[i] = pref[i].max(measure(cell, 10.0)?.min(280.0) + 8.0);
        }
    }
    let sum: f32 = pref.iter().sum();
    let scale = if sum > CONTENT_W {
        CONTENT_W / sum
    } else {
        1.0
    };
    let extra = (CONTENT_W - pref.iter().map(|w| w * scale).sum::<f32>()).max(0.0) / cols as f32;
    let widths: Vec<f32> = pref.iter().map(|w| w * scale + extra).collect();
    for (r, row) in rows.iter().enumerate() {
        let mut cell_lines = Vec::new();
        for (c, cell) in row.iter().enumerate() {
            cell_lines.push(wrap_width(cell, (widths[c] - 8.0).max(12.0), 10.0)?);
        }
        while cell_lines.len() < cols {
            cell_lines.push(vec![String::new()]);
        }
        let mut offsets = vec![0usize; cols];
        loop {
            let remaining = (0..cols)
                .map(|c| cell_lines[c].len().saturating_sub(offsets[c]))
                .max()
                .unwrap_or(0);
            if remaining == 0 {
                break;
            }
            let mut fit = table_lines_that_fit(page.y);
            if fit == 0 {
                if page.y < PAGE_TOP - 0.5 {
                    page.new_page();
                    continue;
                }
                fit = 1;
            }
            let take = remaining.min(fit);
            let row_h = (8.0 + take as f32 * 13.0).max(16.0);
            let bottom = page.y - row_h;
            let mut x = PAGE_LEFT;
            for c in 0..cols {
                let w = widths[c];
                if r == 0 {
                    page.ops().extend(
                        format!("0.945 0.961 0.976 rg {x:.2} {bottom:.2} {w:.2} {row_h:.2} re f\n")
                            .as_bytes(),
                    );
                }
                page.ops().extend(
                    format!("0.796 0.835 0.882 RG {x:.2} {bottom:.2} {w:.2} {row_h:.2} re S\n")
                        .as_bytes(),
                );
                page.ops().extend(b"0.090 0.125 0.200 rg\n");
                let end = (offsets[c] + take).min(cell_lines[c].len());
                let mut baseline = page.y - 12.0;
                for line in &cell_lines[c][offsets[c]..end] {
                    draw_line(page.ops(), line, x + 4.0, baseline, 10.0)?;
                    baseline -= 13.0;
                }
                offsets[c] = end;
                x += w;
            }
            page.y = bottom;
        }
    }
    page.y -= 8.0;
    Ok(())
}
fn paint_rich(content: &str) -> Result<Vec<Vec<u8>>> {
    let mut page = Painter::new();
    for block in blocks(content)? {
        match block {
            Block::Text(line) => {
                for visual in wrap_width(&line, CONTENT_W, 12.0)? {
                    page.ensure(16.0);
                    let baseline = page.y;
                    draw_line(page.ops(), &visual, PAGE_LEFT, baseline, 12.0)?;
                    page.y -= 16.0;
                }
            }
            Block::Table(rows) => paint_table(&mut page, &rows)?,
        }
    }
    Ok(page.pages)
}
fn pdf(content: &str) -> Result<(Vec<u8>, usize)> {
    if document_rich(content)? {
        let pages = paint_rich(content)?;
        let n = pages.len();
        return Ok((assemble(&pages, content.contains(pdf_font::PARTY))?, n));
    }
    let mut visual = Vec::new();
    for line in content.lines() {
        visual.extend(wrap_width(line, CONTENT_W, 12.0)?);
    }
    if visual.is_empty() {
        visual.push(String::new());
    }
    let chunks: Vec<_> = visual.chunks(48).collect();
    let mut contents = Vec::new();
    for chunk in &chunks {
        let mut stream = b"BT /F1 12 Tf 15 TL 42 786 Td\n".to_vec();
        for line in *chunk {
            stream.push(b'(');
            for c in line.chars() {
                let b = pdf_byte(c)?;
                if matches!(b, b'(' | b')' | b'\\') {
                    stream.push(b'\\');
                }
                stream.push(b);
            }
            stream.extend_from_slice(b") Tj T*\n");
        }
        stream.extend_from_slice(b"ET\n");
        contents.push(stream);
    }
    let n = contents.len();
    Ok((assemble(&contents, false)?, n))
}
fn stream_object(data: &[u8], extra: &str) -> Vec<u8> {
    let mut obj = format!("<< /Length {}{extra} >>\nstream\n", data.len()).into_bytes();
    obj.extend(data);
    obj.extend(b"\nendstream");
    obj
}
fn assemble(contents: &[Vec<u8>], emoji: bool) -> Result<Vec<u8>> {
    let face = pdf_font::face();
    let widths = (32..=255)
        .map(|b| face.widths[b].to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let font = format!("<< /Type /Font /Subtype /TrueType /BaseFont /DejaVuSans /Encoding /WinAnsiEncoding /FirstChar 32 /LastChar 255 /Widths [{widths}] /FontDescriptor 4 0 R /ToUnicode 6 0 R >>");
    let descriptor = format!(
        "<< /Type /FontDescriptor /FontName /DejaVuSans /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 5 0 R >>",
        face.bbox[0], face.bbox[1], face.bbox[2], face.bbox[3], face.ascent, face.descent, face.cap_height
    );
    let mut objects = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        Vec::new(),
        font.into_bytes(),
        descriptor.into_bytes(),
        stream_object(face.bytes, &format!(" /Length1 {}", face.bytes.len())),
        stream_object(tounicode(face).as_bytes(), ""),
    ];
    if emoji {
        let pixels = pdf_font::party_popper();
        objects.push(stream_object(pixels, &format!(" /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8", pdf_font::PARTY_PX, pdf_font::PARTY_PX)));
    }
    let resources = if emoji {
        "/Resources << /Font << /F1 3 0 R >> /XObject << /Im1 7 0 R >> >>"
    } else {
        "/Resources << /Font << /F1 3 0 R >> >>"
    };
    let first = objects.len() + 1;
    let mut kids = Vec::new();
    for (i, content) in contents.iter().enumerate() {
        let page_number = first + i * 2;
        let content_number = page_number + 1;
        kids.push(page_number);
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] {resources} /Contents {content_number} 0 R >>").into_bytes());
        objects.push(stream_object(content, ""));
    }
    objects[1] = format!(
        "<< /Type /Pages /Count {} /Kids [{}] >>",
        kids.len(),
        kids.iter()
            .map(|n| format!("{n} 0 R"))
            .collect::<Vec<_>>()
            .join(" ")
    )
    .into_bytes();
    Ok(finish_pdf(objects))
}
fn tounicode(face: &pdf_font::Face) -> String {
    let mut maps = Vec::new();
    for b in 32..=255 {
        let scalar = face.unicode[b];
        if scalar != 0 && face.glyphs[b] != 0 {
            maps.push(format!("<{b:02X}> <{scalar:04X}>"));
        }
    }
    let mut chars = String::new();
    let mut i = 0;
    while i < maps.len() {
        let n = (maps.len() - i).min(100);
        chars.push_str(&format!("{n} beginbfchar\n"));
        for line in &maps[i..i + n] {
            chars.push_str(line);
            chars.push('\n');
        }
        chars.push_str("endbfchar\n");
        i += n;
    }
    format!("/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Ezerbot-WinAnsi-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<20> <FF>\nendcodespacerange\n{chars}endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n")
}
fn finish_pdf(objects: Vec<Vec<u8>>) -> Vec<u8> {
    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = vec![0];
    for (i, obj) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend(obj);
        out.extend(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for n in offsets.iter().skip(1) {
        out.extend(format!("{n:010} 00000 n \n").as_bytes());
    }
    out.extend(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const XML: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>";
fn rels(items: &[(&str, String, String)]) -> String {
    format!("{XML}<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">{}</Relationships>",items.iter().map(|(id,kind,target)|format!("<Relationship Id=\"{id}\" Type=\"{REL}/{kind}\" Target=\"{target}\"/>")).collect::<String>())
}
fn package(
    mut files: Vec<(String, String)>,
    overrides: Vec<(String, String)>,
    main: &str,
) -> Result<Vec<u8>> {
    files.push((
        "_rels/.rels".into(),
        rels(&[("rId1", "officeDocument".into(), main.into())]),
    ));
    files.push(("[Content_Types].xml".into(),format!("{XML}<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>{}</Types>",overrides.iter().map(|(path,kind)|format!("<Override PartName=\"/{path}\" ContentType=\"application/vnd.openxmlformats-officedocument.{kind}+xml\"/>")).collect::<String>())));
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in files {
        zip.start_file(name, SimpleFileOptions::default())
            .map_err(|_| invalid("Impossible d’assembler le document Office."))?;
        zip.write_all(content.as_bytes())?;
    }
    Ok(zip
        .finish()
        .map_err(|_| invalid("Impossible de terminer le document Office."))?
        .into_inner())
}
// One Word paragraph per source line, so Word reflows the text itself. Markdown
// headings and "- " bullets become formatting; a pipe table becomes a real table.
fn docx_paragraph(line: &str) -> String {
    let (level, text) = heading(line);
    let bullet =
        level == 0 && (line.trim_start().starts_with("- ") || line.trim_start().starts_with("* "));
    let text = if bullet {
        &line.trim_start()[2..]
    } else {
        text
    };
    let (size, bold, after) = match level {
        1 => (36, true, 160),
        2 => (30, true, 120),
        3 => (26, true, 100),
        _ => (22, false, 80),
    };
    format!("<w:p><w:pPr><w:spacing w:before=\"{}\" w:after=\"{after}\"/>{}</w:pPr><w:r><w:rPr><w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\" w:cs=\"Arial\"/>{}<w:sz w:val=\"{size}\"/></w:rPr><w:t xml:space=\"preserve\">{}{}</w:t></w:r></w:p>",
        if level > 0 { 240 } else { 0 },
        if bullet { "<w:ind w:left=\"360\" w:hanging=\"240\"/>" } else { "" },
        if bold { "<w:b/>" } else { "" },
        if bullet { "•\u{a0}" } else { "" },
        xml(text))
}
fn docx_table(rows: &[Vec<String>]) -> String {
    let borders = ["top", "left", "bottom", "right", "insideH", "insideV"]
        .iter()
        .map(|edge| {
            format!("<w:{edge} w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"CBD5E1\"/>")
        })
        .collect::<String>();
    let body = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let cells = row
                .iter()
                .map(|cell| {
                    let shade = if r == 0 { "<w:tcPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F1F5F9\"/></w:tcPr>" } else { "" };
                    let bold = if r == 0 { "<w:b/>" } else { "" };
                    format!("<w:tc>{shade}<w:p><w:r><w:rPr><w:rFonts w:ascii=\"Arial\" w:hAnsi=\"Arial\" w:cs=\"Arial\"/>{bold}<w:sz w:val=\"22\"/></w:rPr><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p></w:tc>", xml(cell))
                })
                .collect::<String>();
            format!("<w:tr>{cells}</w:tr>")
        })
        .collect::<String>();
    format!("<w:tbl><w:tblPr><w:tblW w:w=\"5000\" w:type=\"pct\"/><w:tblBorders>{borders}</w:tblBorders></w:tblPr>{body}</w:tbl>")
}
fn docx(content: &str) -> Result<Vec<u8>> {
    let body = blocks(content)?
        .iter()
        .map(|block| match block {
            Block::Text(line) => docx_paragraph(line),
            Block::Table(rows) => docx_table(rows),
        })
        .collect::<String>();
    package(vec![("word/document.xml".into(),format!("{XML}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/></w:sectPr></w:body></w:document>"))],vec![("word/document.xml".into(),"wordprocessingml.document.main".into())],"word/document.xml")
}
fn csv(rows: &[Vec<String>]) -> Vec<u8> {
    // Text cells, not executable spreadsheet formulas. BOM helps Excel detect UTF-8.
    let mut out = String::from("\u{feff}");
    for row in rows {
        out.push_str(
            &row.iter()
                .map(|s| {
                    let value = if formula_like(s) {
                        format!("'{s}")
                    } else {
                        s.clone()
                    };
                    format!("\"{}\"", value.replace('"', "\"\""))
                })
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push_str("\r\n");
    }
    out.into_bytes()
}
/// A cell a spreadsheet could run as a formula. Signed numbers ("-12,50",
/// "+300 €") stay data; any other leading = + - @ tab or CR is neutralised.
fn formula_like(s: &str) -> bool {
    if s.starts_with(['\t', '\r']) {
        return true;
    }
    let t = s.trim_start();
    if t.starts_with(['=', '@']) {
        return true;
    }
    let Some(body) = t.strip_prefix(['+', '-']) else {
        return false;
    };
    let body = body.trim_end();
    let body = body
        .strip_suffix(['%', '€', '$'])
        .unwrap_or(body)
        .trim_end();
    !(body.starts_with(|c: char| c.is_ascii_digit())
        && body
            .chars()
            .all(|c| c.is_ascii_digit() || c.is_whitespace() || c == '.' || c == ','))
}
/// A plain decimal stored as a spreadsheet number: no leading zeros (codes keep
/// them as text), at most 15 significant digits, never a formula.
fn xlsx_number(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    let (int, frac) = digits.split_once('.').unwrap_or((digits, ""));
    !int.is_empty()
        && int.bytes().all(|b| b.is_ascii_digit())
        && frac.bytes().all(|b| b.is_ascii_digit())
        && !(digits.contains('.') && frac.is_empty())
        && (int == "0" || !int.starts_with('0'))
        && int.len() + frac.len() <= 15
}
fn column(mut n: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (n % 26) as u8) as char);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s
}
fn xlsx(rows: &[Vec<String>]) -> Result<Vec<u8>> {
    let data=rows.iter().enumerate().map(|(i,row)|format!("<row r=\"{}\">{}</row>",i+1,row.iter().enumerate().map(|(j,s)|if xlsx_number(s){format!("<c r=\"{}{}\"><v>{s}</v></c>",column(j),i+1)}else{format!("<c r=\"{}{}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",column(j),i+1,xml(s))}).collect::<String>())).collect::<String>();
    package(vec![
        ("xl/workbook.xml".into(),format!("{XML}<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"{REL}\"><sheets><sheet name=\"Données\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>")),
        ("xl/_rels/workbook.xml.rels".into(),rels(&[("rId1","worksheet".into(),"worksheets/sheet1.xml".into())])),
        ("xl/worksheets/sheet1.xml".into(),format!("{XML}<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>{data}</sheetData></worksheet>"))],
        vec![("xl/workbook.xml".into(),"spreadsheetml.sheet.main".into()),("xl/worksheets/sheet1.xml".into(),"spreadsheetml.worksheet".into())],"xl/workbook.xml")
}
fn slides(content: &str) -> Result<Vec<Slide>> {
    let mut raw = vec![String::new()];
    for line in content.lines() {
        if line.trim() == "---" {
            raw.push(String::new());
        } else {
            let last = raw.last_mut().unwrap();
            last.push_str(line);
            last.push('\n');
        }
    }
    if raw.len() > 50 || raw.iter().any(|s| s.trim().is_empty()) {
        return Err(invalid("La présentation doit contenir 1 à 50 diapositives non vides, séparées par une ligne --- ."));
    }
    let mut deck = Vec::new();
    for source in &raw {
        deck.extend(slide_from(source)?);
    }
    if deck.len() > 50 {
        return Err(invalid("La présentation doit contenir 1 à 50 diapositives non vides, séparées par une ligne --- ."));
    }
    Ok(deck)
}
fn slide_from(source: &str) -> Result<Vec<Slide>> {
    let mut items = Vec::new();
    let mut text_lines = 0;
    let mut first = true;
    for block in blocks(source)? {
        match block {
            Block::Text(line) => {
                let width = if first { 50 } else { 80 };
                first = false;
                for wrapped in wrap(&line, width) {
                    text_lines += 1;
                    items.push(Item::Line(wrapped));
                }
            }
            Block::Table(rows) => items.push(Item::Table(rows)),
        }
    }
    if text_lines > 16 {
        return Err(invalid(
            "Une diapositive dépasse 16 lignes. Répartissez son contenu avec une ligne --- .",
        ));
    }
    Ok(paginate_slide(items))
}
const SLIDE_BOTTOM: i64 = 6_858_000;
const SLIDE_ORIGIN: i64 = 400_000;
const TEXT_LINE_EMU: i64 = 360_000;
const TEXT_PAD_EMU: i64 = 200_000;
const TEXT_ALONE_EMU: i64 = 6_000_000;
const TABLE_ROW_EMU: i64 = 400_000;
const TABLE_GAP_EMU: i64 = 60_000;
fn line_units(items: &[Item]) -> usize {
    items
        .iter()
        .map(|item| match item {
            Item::Line(_) => 1,
            Item::Table(rows) => rows.len(),
        })
        .sum()
}
fn frame_bottom(items: &[Item]) -> i64 {
    let mut y = SLIDE_ORIGIN;
    let mut pending = 0usize;
    let mut parts = false;
    let mut bottom = SLIDE_ORIGIN;
    for item in items {
        match item {
            Item::Line(_) => pending += 1,
            Item::Table(rows) => {
                if pending > 0 {
                    let h = TEXT_LINE_EMU * pending as i64 + TEXT_PAD_EMU;
                    bottom = bottom.max(y + h);
                    y += h;
                    pending = 0;
                }
                let h = TABLE_ROW_EMU * rows.len() as i64;
                bottom = bottom.max(y + h);
                y += h + TABLE_GAP_EMU;
                parts = true;
            }
        }
    }
    if pending > 0 {
        let h = if parts {
            TEXT_LINE_EMU * pending as i64 + TEXT_PAD_EMU
        } else {
            TEXT_ALONE_EMU
        };
        bottom = bottom.max(y + h);
    }
    bottom
}
fn y_before_next_table(items: &[Item]) -> i64 {
    let mut y = SLIDE_ORIGIN;
    let mut pending = 0i64;
    for item in items {
        match item {
            Item::Line(_) => pending += 1,
            Item::Table(rows) => {
                if pending > 0 {
                    y += TEXT_LINE_EMU * pending + TEXT_PAD_EMU;
                    pending = 0;
                }
                y += TABLE_ROW_EMU * rows.len() as i64 + TABLE_GAP_EMU;
            }
        }
    }
    if pending > 0 {
        y += TEXT_LINE_EMU * pending + TEXT_PAD_EMU;
    }
    y
}
fn table_rows_that_fit(current: &[Item], available: usize) -> usize {
    let used = line_units(current);
    if used >= 16 || available == 0 {
        return 0;
    }
    let y = y_before_next_table(current);
    if y >= SLIDE_BOTTOM {
        return 0;
    }
    let by_emu = ((SLIDE_BOTTOM - y) / TABLE_ROW_EMU) as usize;
    (16 - used).min(by_emu).min(available)
}
fn paginate_slide(items: Vec<Item>) -> Vec<Slide> {
    let mut slides = Vec::new();
    let mut current = Vec::new();
    for item in items {
        match item {
            Item::Line(line) => {
                current.push(Item::Line(line));
                if line_units(&current) > 16 || frame_bottom(&current) > SLIDE_BOTTOM {
                    let Some(Item::Line(line)) = current.pop() else {
                        continue;
                    };
                    if !current.is_empty() {
                        slides.push(Slide {
                            items: std::mem::take(&mut current),
                        });
                    }
                    current.push(Item::Line(line));
                }
            }
            Item::Table(rows) => {
                let mut start = 0;
                while start < rows.len() {
                    let fit = table_rows_that_fit(&current, rows.len() - start);
                    if fit == 0 {
                        if current.is_empty() {
                            current.push(Item::Table(vec![rows[start].clone()]));
                            start += 1;
                        } else {
                            slides.push(Slide {
                                items: std::mem::take(&mut current),
                            });
                        }
                        continue;
                    }
                    current.push(Item::Table(rows[start..start + fit].to_vec()));
                    start += fit;
                    if start < rows.len() {
                        slides.push(Slide {
                            items: std::mem::take(&mut current),
                        });
                    }
                }
            }
        }
    }
    if !current.is_empty() {
        slides.push(Slide { items: current });
    }
    if slides.is_empty() {
        slides.push(Slide {
            items: vec![Item::Line(String::new())],
        });
    }
    slides
}
const GROUP:&str="<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";
fn text_shape(lines: &[String], y: i64, h: i64, id: i32, title: bool) -> String {
    let paragraphs = lines
        .iter()
        .enumerate()
        .map(|(j, line)| {
            let size = if title && j == 0 { 2800 } else { 1800 };
            format!("<a:p><a:pPr><a:lnSpc><a:spcPts val=\"2800\"/></a:lnSpc><a:spcBef><a:spcPts val=\"0\"/></a:spcBef><a:spcAft><a:spcPts val=\"0\"/></a:spcAft></a:pPr><a:r><a:rPr lang=\"fr-CA\" sz=\"{size}\"><a:solidFill><a:srgbClr val=\"172033\"/></a:solidFill><a:latin typeface=\"Arial\"/></a:rPr><a:t>{}</a:t></a:r></a:p>", xml(line))
        })
        .collect::<String>();
    format!("<p:sp><p:nvSpPr><p:cNvPr id=\"{id}\" name=\"Contenu\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"500000\" y=\"{y}\"/><a:ext cx=\"11192000\" cy=\"{h}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr wrap=\"square\"/><a:lstStyle/>{paragraphs}</p:txBody></p:sp>")
}
fn table_border(tag: &str) -> String {
    format!("<a:{tag} w=\"6350\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:srgbClr val=\"CBD5E1\"/></a:solidFill><a:prstDash val=\"solid\"/></a:{tag}>")
}
fn table_shape(rows: &[Vec<String>], y: i64, id: i32) -> (String, i64) {
    let cols = rows.first().map(Vec::len).unwrap_or(1).max(1);
    let width = 11_192_000i64;
    let col_w = width / cols as i64;
    let row_h = TABLE_ROW_EMU;
    let height = row_h * rows.len() as i64;
    let grid = (0..cols)
        .map(|_| format!("<a:gridCol w=\"{col_w}\"/>"))
        .collect::<String>();
    let borders = ["lnL", "lnR", "lnT", "lnB"]
        .into_iter()
        .map(table_border)
        .collect::<String>();
    let body = rows
        .iter()
        .enumerate()
        .map(|(r, row)| {
            let cells = (0..cols)
                .map(|c| {
                    let text = row.get(c).map(String::as_str).unwrap_or("");
                    let bold = if r == 0 { " b=\"1\"" } else { "" };
                    let fill = if r == 0 { "F1F5F9" } else { "FFFFFF" };
                    format!("<a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"fr-CA\" sz=\"1400\"{bold}><a:solidFill><a:srgbClr val=\"172033\"/></a:solidFill><a:latin typeface=\"Arial\"/></a:rPr><a:t>{}</a:t></a:r></a:p></a:txBody><a:tcPr>{borders}<a:solidFill><a:srgbClr val=\"{fill}\"/></a:solidFill></a:tcPr></a:tc>", xml(text))
                })
                .collect::<String>();
            format!("<a:tr h=\"{row_h}\">{cells}</a:tr>")
        })
        .collect::<String>();
    let frame = format!("<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"{id}\" name=\"Tableau\"/><p:cNvGraphicFramePr><a:graphicFrameLocks noGrp=\"1\"/></p:cNvGraphicFramePr><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"500000\" y=\"{y}\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr/><a:tblGrid>{grid}</a:tblGrid>{body}</a:tbl></a:graphicData></a:graphic></p:graphicFrame>");
    (frame, height)
}
fn slide_tree(slide: &Slide) -> String {
    let mut parts = Vec::new();
    let mut y = SLIDE_ORIGIN;
    let mut id = 2i32;
    let mut pending = Vec::new();
    let mut title = true;
    for item in &slide.items {
        match item {
            Item::Line(line) => pending.push(line.clone()),
            Item::Table(rows) => {
                if !pending.is_empty() {
                    let h = TEXT_LINE_EMU * pending.len() as i64 + TEXT_PAD_EMU;
                    parts.push(text_shape(&pending, y, h, id, title));
                    y += h;
                    id += 1;
                    title = false;
                    pending.clear();
                }
                let (frame, h) = table_shape(rows, y, id);
                parts.push(frame);
                y += h + TABLE_GAP_EMU;
                id += 1;
            }
        }
    }
    if !pending.is_empty() {
        let h = if parts.is_empty() {
            TEXT_ALONE_EMU
        } else {
            TEXT_LINE_EMU * pending.len() as i64 + TEXT_PAD_EMU
        };
        parts.push(text_shape(&pending, y, h, id, title));
    }
    format!("{GROUP}{}", parts.concat())
}
fn pptx(slides: &[Slide]) -> Result<Vec<u8>> {
    let ns=format!("xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"{REL}\"");
    let mut files = vec![];
    let mut types = vec![];
    let mut links = vec![(
        "rId1".to_string(),
        "slideMaster".to_string(),
        "slideMasters/slideMaster1.xml".to_string(),
    )];
    let mut ids = String::new();
    for (i, slide) in slides.iter().enumerate() {
        let n = i + 1;
        ids.push_str(&format!(
            "<p:sldId id=\"{}\" r:id=\"rId{}\"/>",
            256 + i,
            n + 1
        ));
        links.push((
            format!("rId{}", n + 1),
            "slide".into(),
            format!("slides/slide{n}.xml"),
        ));
        let tree = slide_tree(slide);
        let path = format!("ppt/slides/slide{n}.xml");
        types.push((path.clone(), "presentationml.slide".into()));
        files.push((path,format!("{XML}<p:sld {ns}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{tree}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>")));
        files.push((
            format!("ppt/slides/_rels/slide{n}.xml.rels"),
            rels(&[(
                "rId1",
                "slideLayout".into(),
                "../slideLayouts/slideLayout1.xml".into(),
            )]),
        ));
    }
    files.push(("ppt/presentation.xml".into(),format!("{XML}<p:presentation {ns}><p:sldMasterIdLst><p:sldMasterId id=\"2147483648\" r:id=\"rId1\"/></p:sldMasterIdLst><p:sldIdLst>{ids}</p:sldIdLst><p:sldSz cx=\"12192000\" cy=\"6858000\" type=\"screen16x9\"/><p:notesSz cx=\"6858000\" cy=\"9144000\"/></p:presentation>")));
    files.push((
        "ppt/_rels/presentation.xml.rels".into(),
        rels(
            &links
                .iter()
                .map(|(id, k, t)| (id.as_str(), k.clone(), t.clone()))
                .collect::<Vec<_>>(),
        ),
    ));
    let colors="<p:clrMap accent1=\"accent1\" accent2=\"accent2\" accent3=\"accent3\" accent4=\"accent4\" accent5=\"accent5\" accent6=\"accent6\" bg1=\"lt1\" bg2=\"lt2\" folHlink=\"folHlink\" hlink=\"hlink\" tx1=\"dk1\" tx2=\"dk2\"/>";
    files.push(("ppt/slideMasters/slideMaster1.xml".into(),format!("{XML}<p:sldMaster {ns}><p:cSld><p:spTree>{GROUP}</p:spTree></p:cSld>{colors}<p:sldLayoutIdLst><p:sldLayoutId id=\"2147483649\" r:id=\"rId1\"/></p:sldLayoutIdLst><p:txStyles><p:titleStyle/><p:bodyStyle/><p:otherStyle/></p:txStyles></p:sldMaster>")));
    files.push((
        "ppt/slideMasters/_rels/slideMaster1.xml.rels".into(),
        rels(&[
            (
                "rId1",
                "slideLayout".into(),
                "../slideLayouts/slideLayout1.xml".into(),
            ),
            ("rId2", "theme".into(), "../theme/theme1.xml".into()),
        ]),
    ));
    files.push(("ppt/slideLayouts/slideLayout1.xml".into(),format!("{XML}<p:sldLayout {ns} type=\"blank\" preserve=\"1\"><p:cSld name=\"Vide\"><p:spTree>{GROUP}</p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sldLayout>")));
    files.push((
        "ppt/slideLayouts/_rels/slideLayout1.xml.rels".into(),
        rels(&[(
            "rId1",
            "slideMaster".into(),
            "../slideMasters/slideMaster1.xml".into(),
        )]),
    ));
    let palette = [
        ("dk1", "172033"),
        ("lt1", "FFFFFF"),
        ("dk2", "334155"),
        ("lt2", "F1F5F9"),
        ("accent1", "2563EB"),
        ("accent2", "0891B2"),
        ("accent3", "059669"),
        ("accent4", "D97706"),
        ("accent5", "7C3AED"),
        ("accent6", "DB2777"),
        ("hlink", "2563EB"),
        ("folHlink", "7C3AED"),
    ]
    .iter()
    .map(|(k, v)| format!("<a:{k}><a:srgbClr val=\"{v}\"/></a:{k}>"))
    .collect::<String>();
    let fill = "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>";
    let ln = format!("<a:ln w=\"9525\">{fill}<a:prstDash val=\"solid\"/></a:ln>");
    files.push(("ppt/theme/theme1.xml".into(),format!("{XML}<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Ezerbot\"><a:themeElements><a:clrScheme name=\"Ezerbot\">{palette}</a:clrScheme><a:fontScheme name=\"Arial\"><a:majorFont><a:latin typeface=\"Arial\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:majorFont><a:minorFont><a:latin typeface=\"Arial\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:minorFont></a:fontScheme><a:fmtScheme name=\"Simple\"><a:fillStyleLst>{fill}{fill}{fill}</a:fillStyleLst><a:lnStyleLst>{ln}{ln}{ln}</a:lnStyleLst><a:effectStyleLst><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle><a:effectLst/></a:effectStyle></a:effectStyleLst><a:bgFillStyleLst>{fill}{fill}{fill}</a:bgFillStyleLst></a:fmtScheme></a:themeElements></a:theme>")));
    types.extend([
        (
            "ppt/presentation.xml".into(),
            "presentationml.presentation.main".into(),
        ),
        (
            "ppt/slideMasters/slideMaster1.xml".into(),
            "presentationml.slideMaster".into(),
        ),
        (
            "ppt/slideLayouts/slideLayout1.xml".into(),
            "presentationml.slideLayout".into(),
        ),
        ("ppt/theme/theme1.xml".into(), "theme".into()),
    ]);
    package(files, types, "ppt/presentation.xml")
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JsonDiagram {
    nodes: Vec<String>,
    edges: Vec<[usize; 2]>,
}
#[derive(Clone, Copy, PartialEq)]
enum Shape {
    Box,
    Round,
    Diamond,
}
struct Node {
    /// One to three lines, separated by '\n' (Mermaid `<br>`).
    label: String,
    shape: Shape,
}
struct Edge {
    from: usize,
    to: usize,
    label: Option<String>,
    dashed: bool,
    arrow: bool,
}
/// Link label, dashed stroke, arrowhead.
type Link = (Option<String>, bool, bool);
struct Graph {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    horizontal: bool,
}
const MAX_NODES: usize = 20;
const MAX_EDGES: usize = 40;
fn mermaid_error() -> Error {
    invalid("Diagramme Mermaid non pris en charge : commencez par « flowchart TD » ou « flowchart LR », puis des liens comme A[Départ] --> B{Choix} -->|oui| C(Arrivée).")
}
// Subset of Mermaid flowcharts: nodes, shapes, `&` groups, links and link
// labels. Styling, click handlers and subgraph boxes are ignored; nothing in the
// input is executed or linked.
fn mermaid(content: &str) -> Result<Graph> {
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%") && !l.starts_with("```"));
    let first = lines.next().ok_or_else(mermaid_error)?;
    let (header, tail) = first.split_once(';').unwrap_or((first, ""));
    let mut header = header.split_whitespace();
    if !matches!(header.next(), Some("flowchart" | "graph")) {
        return Err(mermaid_error());
    }
    let horizontal = match header.next().unwrap_or("TD") {
        "TD" | "TB" => false,
        "LR" => true,
        _ => return Err(mermaid_error()),
    };
    if header.next().is_some() {
        return Err(mermaid_error());
    }
    let mut graph = Graph {
        nodes: Vec::new(),
        edges: Vec::new(),
        horizontal,
    };
    let mut ids = Vec::<String>::new();
    let statements = tail
        .split(';')
        .chain(lines.flat_map(|l| l.split(';')))
        .map(str::trim);
    for statement in statements {
        let keyword = statement.split_whitespace().next().unwrap_or("");
        if keyword.is_empty()
            || matches!(
                keyword,
                "subgraph"
                    | "end"
                    | "direction"
                    | "classDef"
                    | "class"
                    | "style"
                    | "linkStyle"
                    | "click"
            )
        {
            continue;
        }
        if mermaid_statement(statement, &mut ids, &mut graph).is_none() {
            let shown: String = statement.chars().take(80).collect();
            return Err(invalid(&format!("Ligne Mermaid non comprise : « {shown} ». Utilisez des liens comme A[Départ] --> B{{Choix}} -->|oui| C(Arrivée), ou A & B --> C.")));
        }
        if graph.nodes.len() > MAX_NODES || graph.edges.len() > MAX_EDGES {
            break;
        }
    }
    Ok(graph)
}
/// `A[x] & B --> C -->|oui| D`: every node of a group links to every node of the next.
fn mermaid_statement(s: &str, ids: &mut Vec<String>, graph: &mut Graph) -> Option<()> {
    let mut rest = s;
    let mut previous: Option<(Vec<usize>, Link)> = None;
    loop {
        let mut group = Vec::new();
        let after = loop {
            let (node, after) = mermaid_node(rest.trim_start(), ids, graph)?;
            group.push(node);
            match after.trim_start().strip_prefix('&') {
                Some(next) => rest = next,
                None => break after.trim_start(),
            }
        };
        if let Some((from, (label, dashed, arrow))) = previous.take() {
            for &f in &from {
                for &t in &group {
                    if graph.edges.len() > MAX_EDGES {
                        return Some(());
                    }
                    graph.edges.push(Edge {
                        from: f,
                        to: t,
                        label: label.clone(),
                        dashed,
                        arrow,
                    });
                }
            }
        }
        if after.is_empty() {
            return Some(());
        }
        let (link, after) = mermaid_link(after)?;
        previous = Some((group, link));
        rest = after;
    }
}
fn mermaid_node<'a>(
    s: &'a str,
    ids: &mut Vec<String>,
    graph: &mut Graph,
) -> Option<(usize, &'a str)> {
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    let (id, rest) = s.split_at(end);
    if id.is_empty() {
        return None;
    }
    let mut definition = None;
    for (open, close, shape) in [
        ("((", "))", Shape::Round),
        ("([", "])", Shape::Round),
        ("[[", "]]", Shape::Box),
        ("{{", "}}", Shape::Diamond),
        ("[", "]", Shape::Box),
        ("(", ")", Shape::Round),
        ("{", "}", Shape::Diamond),
    ] {
        if let Some(inner) = rest.strip_prefix(open) {
            let close_at = inner.find(close)?;
            let label = inner[..close_at].trim();
            let label = label
                .strip_prefix('"')
                .and_then(|l| l.strip_suffix('"'))
                .unwrap_or(label);
            let label = ["<br/>", "<br />", "<br>", "<BR>", "<BR/>"]
                .iter()
                .fold(label.to_string(), |l, br| l.replace(br, "\n"));
            definition = Some((label, shape, &inner[close_at + close.len()..]));
            break;
        }
    }
    let index = match ids.iter().position(|known| known == id) {
        Some(index) => index,
        None => {
            ids.push(id.into());
            graph.nodes.push(Node {
                label: id.into(),
                shape: Shape::Box,
            });
            ids.len() - 1
        }
    };
    Some(match definition {
        Some((label, shape, rest)) => {
            graph.nodes[index] = Node { label, shape };
            (index, rest)
        }
        None => (index, rest),
    })
}
/// `-->`, `--->`, `---`, `-.->`, `-.-`, `==>`, `===`, then an optional `|label|`;
/// or the inline forms `-- label -->` and `== label ==>`.
fn mermaid_link(s: &str) -> Option<(Link, &str)> {
    let (dashed, arrow, rest) = if let Some(r) = s.strip_prefix("-.") {
        let r = r.trim_start_matches('.').strip_prefix('-')?;
        match r.strip_prefix('>') {
            Some(r) => (true, true, r),
            None => (true, false, r),
        }
    } else if s.starts_with("--") || s.starts_with("==") {
        let stroke = s.chars().next()?;
        let r = s.trim_start_matches(stroke);
        let count = s.len() - r.len();
        if let Some(r) = r.strip_prefix('>') {
            (false, true, r)
        } else if count >= 3 {
            (false, false, r)
        } else {
            let close = if stroke == '-' { "--" } else { "==" };
            let end = r.find(close)?;
            let label = r[..end].trim();
            let after = r[end..].trim_start_matches(stroke);
            let (arrow, after) = match after.strip_prefix('>') {
                Some(after) => (true, after),
                None => (false, after),
            };
            return Some((
                ((!label.is_empty()).then(|| label.into()), false, arrow),
                after,
            ));
        }
    } else {
        return None;
    };
    if let Some(inner) = rest.trim_start().strip_prefix('|') {
        let end = inner.find('|')?;
        let label = inner[..end].trim();
        return Some((
            ((!label.is_empty()).then(|| label.into()), dashed, arrow),
            &inner[end + 1..],
        ));
    }
    Some(((None, dashed, arrow), rest))
}
fn diagram(content: &str) -> Result<String> {
    let graph = if content.trim_start().starts_with('{') {
        let d:JsonDiagram=serde_json::from_str(content).map_err(|_|invalid("Pour SVG, content doit être un flowchart Mermaid ou un graphe JSON : {\"nodes\":[\"Départ\",\"Arrivée\"],\"edges\":[[0,1]]}."))?;
        if d.edges.iter().flatten().any(|&i| i >= d.nodes.len()) {
            return Err(invalid(
                "Diagramme invalide : un lien désigne un nœud inexistant.",
            ));
        }
        Graph {
            nodes: d
                .nodes
                .into_iter()
                .map(|label| Node {
                    label,
                    shape: Shape::Box,
                })
                .collect(),
            edges: d
                .edges
                .into_iter()
                .map(|[from, to]| Edge {
                    from,
                    to,
                    label: None,
                    dashed: false,
                    arrow: true,
                })
                .collect(),
            horizontal: false,
        }
    } else {
        mermaid(content)?
    };
    if graph.nodes.is_empty()
        || graph.nodes.len() > MAX_NODES
        || graph.edges.len() > MAX_EDGES
        || graph.nodes.iter().any(|n| {
            n.label.trim().is_empty()
                || n.label.lines().count() > 3
                || n.label.lines().any(|l| l.chars().count() > 40)
                || !valid_text(&n.label)
        })
        || graph
            .edges
            .iter()
            .filter_map(|e| e.label.as_ref())
            .any(|l| l.chars().count() > 30 || !valid_text(l) || l.contains('\n'))
    {
        return Err(invalid("Diagramme invalide : 1 à 20 nœuds (3 lignes de 40 caractères au plus), 40 liens maximum (étiquettes de 30 caractères)."));
    }
    Ok(render(&graph))
}
fn mark_back_edges(graph: &Graph, v: usize, state: &mut [u8], back: &mut [bool]) {
    state[v] = 1;
    for (i, e) in graph.edges.iter().enumerate().filter(|(_, e)| e.from == v) {
        match state[e.to] {
            0 => mark_back_edges(graph, e.to, state, back),
            1 => back[i] = true,
            _ => {}
        }
    }
    state[v] = 2;
}
type Point = (f64, f64);
#[derive(Clone, Copy)]
enum Segment {
    Line(Point, Point),
    Curve(Point, Point, Point, Point),
}
impl Segment {
    fn at(&self, t: f64) -> Point {
        match *self {
            Segment::Line(a, b) => (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t),
            Segment::Curve(a, b, c, d) => {
                let u = 1.0 - t;
                let f = |a: f64, b: f64, c: f64, d: f64| {
                    u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d
                };
                (f(a.0, b.0, c.0, d.0), f(a.1, b.1, c.1, d.1))
            }
        }
    }
    fn reversed(self) -> Self {
        match self {
            Segment::Line(a, b) => Segment::Line(b, a),
            Segment::Curve(a, b, c, d) => Segment::Curve(d, c, b, a),
        }
    }
}
/// Screen rectangle by centre and size.
#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}
impl Rect {
    fn overlaps(&self, o: &Rect) -> bool {
        (self.x - o.x).abs() * 2.0 < self.w + o.w && (self.y - o.y).abs() * 2.0 < self.h + o.h
    }
}
struct Routed {
    segments: Vec<Segment>,
    label: Option<Rect>,
}
struct Layout {
    width: f64,
    height: f64,
    nodes: Vec<Rect>,
    edges: Vec<Routed>,
}
fn node_size(node: &Node) -> (f64, f64) {
    let longest = node
        .label
        .lines()
        .map(|l| l.chars().count())
        .max()
        .unwrap_or(0);
    let lines = node.label.lines().count().max(1) as f64;
    let w = (longest as f64 * 10.0 + 40.0).clamp(120.0, 440.0);
    let h = 56.0 + 20.0 * (lines - 1.0);
    if node.shape == Shape::Diamond {
        (w + 60.0, h + 20.0)
    } else {
        (w, h)
    }
}
fn label_width(label: &str) -> f64 {
    label.chars().count() as f64 * 7.5 + 14.0
}
// Layered (Sugiyama-style) layout. Back edges found by DFS are routed reversed;
// an edge spanning several ranks gets a virtual waypoint on every rank between,
// so it travels through a free slot instead of under a node. Barycentre sweeps
// order each rank. Curves only bend inside the gaps between ranks, and labels sit
// in those gaps too, so neither can cover a node.
fn layout(graph: &Graph) -> Layout {
    const SLOT: f64 = 16.0;
    const SPACING: f64 = 48.0;
    const MARGIN: f64 = 40.0;
    let n = graph.nodes.len();
    let mut state = vec![0; n];
    let mut back = vec![false; graph.edges.len()];
    for v in 0..n {
        if state[v] == 0 {
            mark_back_edges(graph, v, &mut state, &mut back);
        }
    }
    // (upper, lower) node of each edge in rank order.
    let ends: Vec<(usize, usize)> = graph
        .edges
        .iter()
        .zip(&back)
        .map(|(e, &b)| if b { (e.to, e.from) } else { (e.from, e.to) })
        .collect();
    let mut rank = vec![0usize; n];
    for _ in 0..n {
        for (e, &b) in graph.edges.iter().zip(&back) {
            if !b {
                rank[e.to] = rank[e.to].max(rank[e.from] + 1);
            }
        }
    }
    let ranks = rank.iter().max().unwrap() + 1;
    // Sizes on the rank axis ("along") and within a rank ("across").
    let sizes: Vec<(f64, f64)> = graph.nodes.iter().map(node_size).collect();
    let (across, along): (Vec<f64>, Vec<f64>) = sizes
        .iter()
        .map(|&(w, h)| if graph.horizontal { (h, w) } else { (w, h) })
        .unzip();
    let self_loop: Vec<bool> = (0..n)
        .map(|v| graph.edges.iter().any(|e| e.from == v && e.to == v))
        .collect();
    // Items 0..n are nodes; the rest are waypoints. chains[e] lists an edge's items.
    let mut item_rank = rank.clone();
    let mut layers = vec![Vec::new(); ranks];
    for v in 0..n {
        layers[rank[v]].push(v);
    }
    let mut chains = Vec::with_capacity(graph.edges.len());
    for &(upper, lower) in &ends {
        let mut chain = vec![upper];
        if upper != lower {
            let between = rank[upper] + 1..rank[lower];
            for (r, layer) in layers
                .iter_mut()
                .enumerate()
                .take(between.end)
                .skip(between.start)
            {
                item_rank.push(r);
                layer.push(item_rank.len() - 1);
                chain.push(item_rank.len() - 1);
            }
            chain.push(lower);
        }
        chains.push(chain);
    }
    let items = item_rank.len();
    let (mut above, mut below) = (vec![Vec::new(); items], vec![Vec::new(); items]);
    for chain in &chains {
        for pair in chain.windows(2) {
            below[pair[0]].push(pair[1]);
            above[pair[1]].push(pair[0]);
        }
    }
    for sweep in 0..6 {
        let down = sweep % 2 == 0;
        let order: Vec<usize> = if down {
            (1..ranks).collect()
        } else {
            (0..ranks.saturating_sub(1)).rev().collect()
        };
        for r in order {
            let reference = &layers[if down { r - 1 } else { r + 1 }];
            let mut position = vec![f64::NAN; items];
            for (i, &item) in reference.iter().enumerate() {
                position[item] = i as f64;
            }
            let mut keyed: Vec<(f64, usize)> = layers[r]
                .iter()
                .enumerate()
                .map(|(i, &item)| {
                    let near = if down { &above[item] } else { &below[item] };
                    let known: Vec<f64> = near
                        .iter()
                        .map(|&o| position[o])
                        .filter(|p| !p.is_nan())
                        .collect();
                    let key = if known.is_empty() {
                        i as f64
                    } else {
                        known.iter().sum::<f64>() / known.len() as f64
                    };
                    (key, item)
                })
                .collect();
            keyed.sort_by(|a, b| a.0.total_cmp(&b.0));
            layers[r] = keyed.into_iter().map(|(_, item)| item).collect();
        }
    }
    let allocation = |item: usize| {
        if item < n {
            across[item] + if self_loop[item] { 56.0 } else { 0.0 }
        } else {
            SLOT
        }
    };
    let layer_width = |layer: &Vec<usize>| {
        layer.iter().map(|&i| allocation(i)).sum::<f64>()
            + SPACING * layer.len().saturating_sub(1) as f64
    };
    let total_across = layers.iter().map(layer_width).fold(0.0, f64::max);
    let mut item_across = vec![0.0; items];
    for layer in &layers {
        let mut cursor = MARGIN + (total_across - layer_width(layer)) / 2.0;
        for &item in layer {
            item_across[item] = cursor
                + if item < n {
                    across[item] / 2.0
                } else {
                    SLOT / 2.0
                };
            cursor += allocation(item) + SPACING;
        }
    }
    let widest_label = graph
        .edges
        .iter()
        .filter_map(|e| e.label.as_deref())
        .map(label_width)
        .fold(0.0, f64::max);
    let gap = if graph.horizontal {
        (widest_label + 44.0).max(84.0)
    } else {
        84.0
    };
    let half: Vec<f64> = layers
        .iter()
        .map(|layer| {
            layer
                .iter()
                .filter(|&&i| i < n)
                .map(|&v| along[v] / 2.0)
                .fold(28.0, f64::max)
        })
        .collect();
    let mut centre = vec![0.0; ranks];
    let mut cursor = MARGIN;
    for r in 0..ranks {
        centre[r] = cursor + half[r];
        cursor += 2.0 * half[r] + gap;
    }
    let total_along = cursor - gap + MARGIN;
    let screen = |across: f64, along: f64| {
        if graph.horizontal {
            (along, across)
        } else {
            (across, along)
        }
    };
    let nodes: Vec<Rect> = (0..n)
        .map(|v| {
            let (x, y) = screen(item_across[v], centre[rank[v]]);
            Rect {
                x,
                y,
                w: sizes[v].0,
                h: sizes[v].1,
            }
        })
        .collect();
    let mut placed_labels: Vec<Rect> = Vec::new();
    let mut parallel: Vec<(usize, usize)> = Vec::new();
    let mut edges = Vec::with_capacity(graph.edges.len());
    for (e, edge) in graph.edges.iter().enumerate() {
        let (upper, lower) = ends[e];
        let mut segments = Vec::new();
        if upper == lower {
            let (a, l) = (
                item_across[upper] + across[upper] / 2.0,
                centre[rank[upper]],
            );
            segments.push(Segment::Curve(
                screen(a, l - 10.0),
                screen(a + 44.0, l - 34.0),
                screen(a + 44.0, l + 34.0),
                screen(a, l + 10.0),
            ));
        } else {
            // Parallel edges between the same two nodes fan out slightly.
            let twins = parallel.iter().filter(|&&p| p == (upper, lower)).count();
            parallel.push((upper, lower));
            let offset = [0.0, 14.0, -14.0, 28.0, -28.0][twins.min(4)];
            let mut points = vec![(
                item_across[upper] + offset,
                centre[rank[upper]] + along[upper] / 2.0,
            )];
            for &waypoint in &chains[e][1..chains[e].len() - 1] {
                let r = item_rank[waypoint];
                points.push((item_across[waypoint], centre[r] - half[r]));
                points.push((item_across[waypoint], centre[r] + half[r]));
            }
            points.push((
                item_across[lower] + offset,
                centre[rank[lower]] - along[lower] / 2.0,
            ));
            for (i, pair) in points.windows(2).enumerate() {
                let (p, q) = (pair[0], pair[1]);
                segments.push(if i % 2 == 1 {
                    Segment::Line(screen(p.0, p.1), screen(q.0, q.1))
                } else {
                    let m = (p.1 + q.1) / 2.0;
                    Segment::Curve(
                        screen(p.0, p.1),
                        screen(p.0, m),
                        screen(q.0, m),
                        screen(q.0, q.1),
                    )
                });
            }
            if back[e] {
                segments = segments.into_iter().rev().map(Segment::reversed).collect();
            }
        }
        // The label sits on the first bend after the edge's own source, inside a gap.
        let label = edge.label.as_deref().map(|text| {
            let w = label_width(text);
            let first = segments[0];
            [0.5, 0.3, 0.7, 0.2, 0.8]
                .iter()
                .map(|&t| {
                    let (x, y) = first.at(t);
                    Rect { x, y, w, h: 22.0 }
                })
                .find(|r| !placed_labels.iter().chain(&nodes).any(|o| o.overlaps(r)))
                .unwrap_or_else(|| {
                    let (x, y) = first.at(0.5);
                    Rect { x, y, w, h: 22.0 }
                })
        });
        placed_labels.extend(label);
        edges.push(Routed { segments, label });
    }
    let (width, height) = screen(total_across + 2.0 * MARGIN, total_along);
    Layout {
        width,
        height,
        nodes,
        edges,
    }
}
fn render(graph: &Graph) -> String {
    let layout = layout(graph);
    let (w, h) = (layout.width, layout.height);
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w:.0} {h:.0}\" width=\"{w:.0}\" height=\"{h:.0}\" font-family=\"Helvetica,Arial,sans-serif\"><rect width=\"{w:.0}\" height=\"{h:.0}\" fill=\"#fff\"/><defs><marker id=\"arrow\" markerWidth=\"10\" markerHeight=\"10\" refX=\"9\" refY=\"5\" orient=\"auto\"><path d=\"M0 0 L10 5 L0 10Z\" fill=\"#64748b\"/></marker></defs>");
    let mut labels = String::new();
    for (edge, routed) in graph.edges.iter().zip(&layout.edges) {
        let start = match routed.segments[0] {
            Segment::Line(a, _) | Segment::Curve(a, ..) => a,
        };
        let mut path = format!("M{:.0} {:.0}", start.0, start.1);
        for segment in &routed.segments {
            path.push_str(&match *segment {
                Segment::Line(_, b) => format!(" L{:.0} {:.0}", b.0, b.1),
                Segment::Curve(_, b, c, d) => format!(
                    " C{:.0} {:.0} {:.0} {:.0} {:.0} {:.0}",
                    b.0, b.1, c.0, c.1, d.0, d.1
                ),
            });
        }
        svg.push_str(&format!(
            "<path d=\"{path}\" fill=\"none\" stroke=\"#64748b\" stroke-width=\"2\"{}{}/>",
            if edge.dashed {
                " stroke-dasharray=\"6 5\""
            } else {
                ""
            },
            if edge.arrow {
                " marker-end=\"url(#arrow)\""
            } else {
                ""
            }
        ));
        if let (Some(text), Some(r)) = (&edge.label, routed.label) {
            labels.push_str(&format!("<rect x=\"{:.0}\" y=\"{:.0}\" width=\"{:.0}\" height=\"22\" rx=\"4\" fill=\"#fff\" stroke=\"#cbd5e1\"/><text x=\"{:.0}\" y=\"{:.0}\" text-anchor=\"middle\" font-size=\"13\" fill=\"#334155\">{}</text>", r.x - r.w / 2.0, r.y - 11.0, r.w, r.x, r.y + 4.5, xml(text)));
        }
    }
    for (node, r) in graph.nodes.iter().zip(&layout.nodes) {
        let (x, y, l, t) = (r.x, r.y, r.x - r.w / 2.0, r.y - r.h / 2.0);
        svg.push_str(&match node.shape {
            Shape::Diamond => format!("<polygon points=\"{x:.0},{t:.0} {:.0},{y:.0} {x:.0},{:.0} {l:.0},{y:.0}\" fill=\"#fef3c7\" stroke=\"#d97706\" stroke-width=\"1.5\"/>", l + r.w, t + r.h),
            shape => format!("<rect x=\"{l:.0}\" y=\"{t:.0}\" width=\"{:.0}\" height=\"{:.0}\" rx=\"{}\" fill=\"#eff6ff\" stroke=\"#2563eb\" stroke-width=\"1.5\"/>", r.w, r.h, if shape == Shape::Round { 28 } else { 8 }),
        });
        let lines: Vec<&str> = node.label.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            svg.push_str(&format!(
                "<text x=\"{x:.0}\" y=\"{:.0}\" text-anchor=\"middle\" font-size=\"16\" fill=\"#172033\">{}</text>",
                y + 5.5 + (i as f64 - (lines.len() as f64 - 1.0) / 2.0) * 20.0,
                xml(line)
            ));
        }
    }
    // Labels are placed in gaps between ranks, never over a node.
    svg.push_str(&labels);
    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWCASE: &str =
        "Été, ça va 🎉\n\n| Nom | Coût |\n| --- | --- |\n| Thé | 12 € |\n| Ça | 3 |\n";

    fn winansi_char(b: u8) -> char {
        match b {
            0x20..=0x7e | 0xa0..=0xff => b as char,
            0x80 => '€',
            0x82 => '‚',
            0x83 => 'ƒ',
            0x84 => '„',
            0x85 => '…',
            0x86 => '†',
            0x87 => '‡',
            0x88 => 'ˆ',
            0x89 => '‰',
            0x8a => 'Š',
            0x8b => '‹',
            0x8c => 'Œ',
            0x8e => 'Ž',
            0x91 => '‘',
            0x92 => '’',
            0x93 => '“',
            0x94 => '”',
            0x95 => '•',
            0x96 => '–',
            0x97 => '—',
            0x98 => '˜',
            0x99 => '™',
            0x9a => 'š',
            0x9b => '›',
            0x9c => 'œ',
            0x9e => 'ž',
            0x9f => 'Ÿ',
            _ => '\u{FFFD}',
        }
    }

    fn content_streams(bytes: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut rest = bytes;
        while let Some(i) = rest.windows(7).position(|w| w == b"stream\n") {
            let start = i + 7;
            let Some(j) = rest[start..].windows(9).position(|w| w == b"endstream") else {
                break;
            };
            let stream = &rest[start..start + j];
            if stream.windows(2).any(|w| w == b"Tj") || stream.windows(4).any(|w| w == b" re ") {
                out.push(stream.to_vec());
            }
            rest = &rest[start + j..];
        }
        out
    }

    /// Text actually shown by Tj operators, decoded as WinAnsi (the PDF encoding).
    fn pdf_shown(bytes: &[u8]) -> String {
        let mut text = String::new();
        for stream in content_streams(bytes) {
            let mut i = 0;
            while i < stream.len() {
                if stream[i] != b'(' {
                    i += 1;
                    continue;
                }
                i += 1;
                let mut raw = Vec::new();
                while i < stream.len() {
                    match stream[i] {
                        b'\\' => {
                            i += 1;
                            if i < stream.len() {
                                raw.push(stream[i]);
                                i += 1;
                            }
                        }
                        b')' => {
                            i += 1;
                            break;
                        }
                        c => {
                            raw.push(c);
                            i += 1;
                        }
                    }
                }
                let mut j = i;
                while j < stream.len() && stream[j].is_ascii_whitespace() {
                    j += 1;
                }
                if stream.get(j..j + 2) == Some(b"Tj") {
                    text.extend(raw.iter().copied().map(winansi_char));
                    text.push('\n');
                }
            }
        }
        text
    }

    fn office_part(bytes: &[u8], name: &str) -> String {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        std::io::Read::read_to_string(&mut archive.by_name(name).unwrap(), &mut xml).unwrap();
        xml
    }

    #[test]
    fn pdf_docx_and_pptx_show_accents_emoji_and_a_real_table() {
        let pdf = generate(&Request {
            name: "Bilan".into(),
            format: "pdf".into(),
            content: SHOWCASE.into(),
        })
        .unwrap_or_else(|e| panic!("pdf: {e:?}"));
        let shown = pdf_shown(&pdf.bytes);
        for piece in ["Été", "ça", "Nom", "Coût", "Thé", "12 €", "Ça", "3"] {
            assert!(shown.contains(piece), "extraction « {piece} » : {shown}");
        }
        assert!(
            !shown.contains('\u{FFFD}'),
            "glyphe manquant dans le texte : {shown}"
        );
        assert!(
            !shown.contains("| --- |"),
            "le tableau est encore du Markdown : {shown}"
        );
        let streams = content_streams(&pdf.bytes);
        let rects: usize = streams
            .iter()
            .map(|s| s.windows(4).filter(|w| *w == b" re ").count())
            .sum();
        assert!(rects >= 6, "cellules dessinées : {rects}");
        assert!(
            pdf.bytes.windows(15).any(|w| w == b"/Subtype /Image"),
            "l’emoji doit être dessiné, pas absent"
        );
        assert!(
            pdf.bytes.windows(10).any(|w| w == b"/FontFile2"),
            "police embarquée"
        );
        assert!(!pdf.bytes.windows(17).any(|w| w == b"/BaseFont /Courier"));
        assert!(pdf.preview.contains('🎉'), "{}", pdf.preview);
        assert!(!pdf.preview.contains('\u{FFFD}') && !pdf.preview.contains('□'));

        let docx = generate(&Request {
            name: "Bilan".into(),
            format: "docx".into(),
            content: SHOWCASE.into(),
        })
        .unwrap();
        let document = office_part(&docx.bytes, "word/document.xml");
        assert!(
            document.contains("<w:tbl>") && document.contains("<w:tc>"),
            "{document}"
        );
        for piece in ["Été", "ça", "🎉", "Nom", "Coût", "Thé", "12 €", "Ça"] {
            assert!(document.contains(piece), "docx « {piece} »");
        }
        assert!(!document.contains("| --- |"));
        assert!(docx.preview.contains('🎉') && !docx.preview.contains('\u{FFFD}'));

        let pptx = generate(&Request {
            name: "Bilan".into(),
            format: "pptx".into(),
            content: SHOWCASE.into(),
        })
        .unwrap();
        let slide = office_part(&pptx.bytes, "ppt/slides/slide1.xml");
        assert!(
            slide.contains("<a:tbl>") && slide.contains("<a:tc>"),
            "{slide}"
        );
        for piece in ["Été", "ça", "🎉", "Nom", "Coût", "Thé", "12 €", "Ça"] {
            assert!(slide.contains(piece), "pptx « {piece} »");
        }
        assert!(!slide.contains("| --- |"));
        assert!(pptx.preview.contains('🎉'));

        let classic = generate(&Request {
            name: "Accent".into(),
            format: "pdf".into(),
            content: "Été à Québec — Cœur, coût : 12 €".into(),
        })
        .unwrap();
        let classic_text = pdf_shown(&classic.bytes);
        assert!(
            classic_text.contains("Été") && classic_text.contains("Cœur"),
            "{classic_text}"
        );
        assert!(!classic_text.contains('\u{FFFD}'));
        assert!(classic.bytes.starts_with(b"%PDF-"));
        assert!(classic.bytes.windows(7).any(|w| w == b") Tj T*"));
        assert!(!classic.bytes.windows(15).any(|w| w == b"/Subtype /Image"));

        let Err(Error::InvalidInput(message)) = generate(&Request {
            name: "Hors".into(),
            format: "pdf".into(),
            content: "Été → suite".into(),
        }) else {
            panic!("un caractère hors couverture doit échouer");
        };
        assert!(message.contains("« → » (U+2192)"), "{message}");
        assert!(
            message.contains("n’est pas produit") || message.contains("hors"),
            "{message}"
        );

        let long = "ç".repeat(90);
        let deck = format!(
            "{}🎉\n| Nom | Détail |\n| --- | --- |\n| Thé | {long} |\n",
            "a".repeat(50)
        );
        let slides = generate(&Request {
            name: "Long".into(),
            format: "pptx".into(),
            content: deck,
        })
        .unwrap();
        let slide = office_part(&slides.bytes, "ppt/slides/slide1.xml");
        assert!(slide.contains('🎉'), "l’emoji de fin de titre doit rester");
        assert!(
            slide.split("<a:tc>").any(|cell| cell.contains(&long)),
            "la cellule ne doit pas être coupée"
        );
    }

    fn dejavu_width(text: &str, size: f32) -> f32 {
        let face = pdf_font::face();
        text.chars()
            .map(|c| {
                let b = pdf_font::encode(c).expect("glyphe") as usize;
                face.widths[b] as f32 * size / 1000.0
            })
            .sum()
    }

    fn shape_bottoms(xml: &str) -> Vec<i64> {
        let mut bottoms = Vec::new();
        let mut rest = xml;
        while let Some(i) = rest.find("<a:xfrm>") {
            rest = &rest[i + 8..];
            let Some(end) = rest.find("</a:xfrm>") else {
                break;
            };
            let xfrm = &rest[..end];
            rest = &rest[end..];
            let Some(y) = attr_i64(xfrm, "y=\"") else {
                continue;
            };
            let Some(cy) = attr_i64(xfrm, "cy=\"") else {
                continue;
            };
            bottoms.push(y + cy);
        }
        bottoms
    }

    fn attr_i64(xml: &str, key: &str) -> Option<i64> {
        let start = xml.find(key)? + key.len();
        let end = xml[start..].find('"')? + start;
        xml[start..end].parse().ok()
    }

    fn pdf_baselines(bytes: &[u8]) -> Vec<f32> {
        let mut out = Vec::new();
        for stream in content_streams(bytes) {
            let text = String::from_utf8_lossy(&stream);
            let mut rest = text.as_ref();
            while let Some(i) = rest.find(" Tm ") {
                let y = rest[..i]
                    .split_whitespace()
                    .next_back()
                    .and_then(|token| token.parse::<f32>().ok());
                if let Some(y) = y {
                    out.push(y);
                }
                rest = &rest[i + 4..];
            }
        }
        out
    }

    struct PdfRun {
        x: f32,
        y: f32,
        size: f32,
        text: String,
    }

    struct PdfBox {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    }

    struct PdfImage {
        x: f32,
        y: f32,
        size: f32,
    }

    fn pdf_text_runs(bytes: &[u8]) -> Vec<PdfRun> {
        let mut out = Vec::new();
        for stream in content_streams(bytes) {
            let mut i = 0;
            while let Some(rel) = stream[i..].windows(7).position(|w| w == b"BT /F1 ") {
                i += rel + 7;
                let Some(tf) = stream[i..].windows(4).position(|w| w == b" Tf ") else {
                    break;
                };
                let size = std::str::from_utf8(&stream[i..i + tf])
                    .unwrap_or("")
                    .trim()
                    .parse()
                    .unwrap_or(0.0);
                i += tf + 4;
                let Some(tm) = stream[i..].windows(5).position(|w| w == b" Tm (") else {
                    break;
                };
                let header = std::str::from_utf8(&stream[i..i + tm]).unwrap_or("");
                let nums: Vec<f32> = header
                    .split_whitespace()
                    .filter_map(|token| token.parse().ok())
                    .collect();
                let (x, y) = if nums.len() >= 2 {
                    (nums[nums.len() - 2], nums[nums.len() - 1])
                } else {
                    (0.0, 0.0)
                };
                i += tm + 5;
                let mut shown = String::new();
                while i < stream.len() {
                    match stream[i] {
                        b'\\' => {
                            i += 1;
                            if i < stream.len() {
                                shown.push(winansi_char(stream[i]));
                                i += 1;
                            }
                        }
                        b')' => {
                            i += 1;
                            break;
                        }
                        c => {
                            shown.push(winansi_char(c));
                            i += 1;
                        }
                    }
                }
                if !shown.is_empty() {
                    out.push(PdfRun {
                        x,
                        y,
                        size,
                        text: shown,
                    });
                }
            }
        }
        out
    }

    fn pdf_images(bytes: &[u8]) -> Vec<PdfImage> {
        let mut out = Vec::new();
        for stream in content_streams(bytes) {
            let text = String::from_utf8_lossy(&stream);
            for line in text.lines() {
                let Some(idx) = line.find(" cm /Im1 Do") else {
                    continue;
                };
                let nums: Vec<f32> = line[..idx]
                    .split_whitespace()
                    .filter_map(|token| token.parse().ok())
                    .collect();
                if nums.len() >= 6 {
                    let n = nums.len();
                    out.push(PdfImage {
                        size: nums[n - 6],
                        x: nums[n - 2],
                        y: nums[n - 1],
                    });
                }
            }
        }
        out
    }

    fn box_containing(boxes: &[PdfBox], x: f32, y: f32) -> Option<&PdfBox> {
        boxes.iter().find(|b| {
            x + 0.2 >= b.x && x <= b.x + b.w + 0.2 && y + 0.2 >= b.y && y <= b.y + b.h + 0.2
        })
    }

    fn pdf_stroke_boxes(bytes: &[u8]) -> Vec<PdfBox> {
        let mut out = Vec::new();
        for stream in content_streams(bytes) {
            let text = String::from_utf8_lossy(&stream);
            for line in text.lines() {
                let Some(idx) = line.find(" re S") else {
                    continue;
                };
                let nums: Vec<f32> = line[..idx]
                    .split_whitespace()
                    .filter_map(|token| token.parse().ok())
                    .collect();
                if nums.len() >= 4 {
                    let n = nums.len();
                    out.push(PdfBox {
                        x: nums[n - 4],
                        y: nums[n - 3],
                        w: nums[n - 2],
                        h: nums[n - 1],
                    });
                }
            }
        }
        out
    }

    fn md_cells(cell: &str, cols: usize) -> String {
        format!("|{}|", vec![cell; cols].join("|"))
    }

    /// A header of ~63k bars must be refused before `blocks` pads every row.
    #[test]
    fn markdown_table_bounds_columns_and_rows_before_aligning() {
        let wide = md_cells("x", 51);
        let Err(Error::InvalidInput(message)) = generate(&Request {
            name: "Large".into(),
            format: "pdf".into(),
            content: format!("{wide}\n| --- |\n| x |\n"),
        }) else {
            panic!("51 colonnes doivent être refusées");
        };
        assert!(
            message.contains("50") && message.contains("1 000"),
            "{message}"
        );

        let mut bomb = "|".repeat(63_000);
        bomb.push('\n');
        bomb.push_str("| --- |\n");
        for _ in 0..400 {
            bomb.push_str("|x|\n");
        }
        let Err(Error::InvalidInput(message)) = generate(&Request {
            name: "Bombe".into(),
            format: "docx".into(),
            content: bomb,
        }) else {
            panic!("63 000 barres ne doivent pas être alignées");
        };
        assert!(message.contains("50"), "{message}");

        let mut tall = String::from("| A |\n| --- |\n");
        for _ in 0..1000 {
            tall.push_str("| x |\n");
        }
        let Err(Error::InvalidInput(message)) = generate(&Request {
            name: "Haut".into(),
            format: "pptx".into(),
            content: tall,
        }) else {
            panic!("1 001 lignes doivent être refusées");
        };
        assert!(message.contains("1 000"), "{message}");

        let fit = format!(
            "{}\n{}\n{}\n",
            md_cells("x", 50),
            md_cells("---", 50),
            md_cells("x", 50)
        );
        let pdf = generate(&Request {
            name: "Cinquante".into(),
            format: "pdf".into(),
            content: fit,
        })
        .unwrap_or_else(|e| panic!("50 colonnes restent acceptées : {e:?}"));
        assert!(pdf.bytes.starts_with(b"%PDF-"));
        assert!(pdf_shown(&pdf.bytes).contains('x'));

        let mut rows = String::from("| A |\n| --- |\n");
        for _ in 0..999 {
            rows.push_str("| x |\n");
        }
        generate(&Request {
            name: "Mille".into(),
            format: "pdf".into(),
            content: rows,
        })
        .unwrap_or_else(|e| panic!("1 000 lignes restent acceptées : {e:?}"));
    }

    #[test]
    fn plain_pdf_wraps_by_dejavu_width() {
        let wide = "W".repeat(70);
        let overflow = 42.0 + dejavu_width(&wide, 12.0);
        assert!(
            overflow > 595.0,
            "70 W en DejaVu 12 pt doivent dépasser la page : {overflow}"
        );
        let pdf = generate(&Request {
            name: "Large".into(),
            format: "pdf".into(),
            content: wide,
        })
        .unwrap();
        let shown = pdf_shown(&pdf.bytes);
        let lines: Vec<&str> = shown.lines().filter(|line| line.contains('W')).collect();
        assert!(lines.len() > 1, "la ligne doit être coupée : {lines:?}");
        assert_eq!(lines.concat().chars().filter(|c| *c == 'W').count(), 70);
        for line in lines {
            let end = 42.0 + dejavu_width(line, 12.0);
            assert!(end <= 595.0, "la ligne dépasse 595 pt ({end}) : {line}");
        }
    }

    #[test]
    fn pdf_table_row_taller_than_a_page_keeps_baselines_in_the_margin() {
        let cell = "W".repeat(4_000);
        let pdf = generate(&Request {
            name: "Rang".into(),
            format: "pdf".into(),
            content: format!("| Nom |\n| --- |\n| {cell} |\n"),
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
        let baselines = pdf_baselines(&pdf.bytes);
        assert!(!baselines.is_empty(), "aucun texte positionné");
        for y in &baselines {
            assert!(*y >= 42.0, "baseline sous la marge : {y}");
        }
        assert!(
            pdf.pages > 1,
            "le rang doit être réparti sur plusieurs pages"
        );
        assert_eq!(
            pdf_shown(&pdf.bytes).chars().filter(|c| *c == 'W').count(),
            4_000
        );
    }

    fn assert_table_ink_inside_columns(cell: &str) {
        let cols = 50;
        let pdf = generate(&Request {
            name: "Large".into(),
            format: "pdf".into(),
            content: format!(
                "{}\n{}\n{}\n",
                md_cells(cell, cols),
                md_cells("---", cols),
                md_cells(cell, cols)
            ),
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
        let runs = pdf_text_runs(&pdf.bytes);
        let boxes = pdf_stroke_boxes(&pdf.bytes);
        assert!(!runs.is_empty(), "aucun texte de cellule");
        assert!(!boxes.is_empty(), "aucune cellule dessinée");
        assert_ink_inside(&boxes, &runs, &pdf_images(&pdf.bytes));
        let needle = cell.chars().next().expect("cellule");
        assert_eq!(
            pdf_shown(&pdf.bytes)
                .chars()
                .filter(|c| *c == needle)
                .count(),
            cols * 2 * cell.chars().filter(|c| *c == needle).count()
        );
    }

    #[test]
    fn pdf_wide_table_of_i_stays_inside_its_column() {
        assert_table_ink_inside_columns("iiiiiiii");
    }

    #[test]
    fn pdf_wide_table_wide_glyph_stays_inside_its_column() {
        assert_table_ink_inside_columns("WWWWWWWW");
    }

    fn assert_ink_inside(boxes: &[PdfBox], runs: &[PdfRun], images: &[PdfImage]) {
        for run in runs {
            let Some(cell_box) = box_containing(boxes, run.x, run.y) else {
                panic!(
                    "texte hors de toute cellule à ({}, {}) : {}",
                    run.x, run.y, run.text
                );
            };
            let end = run.x + dejavu_width(&run.text, run.size);
            let right = cell_box.x + cell_box.w;
            assert!(
                end <= right + 0.15,
                "le texte dépasse la colonne ({end} > {right}) : {}",
                run.text
            );
        }
        for image in images {
            let Some(cell_box) = box_containing(boxes, image.x, image.y) else {
                panic!("image hors de toute cellule à ({}, {})", image.x, image.y);
            };
            let end = image.x + image.size + 1.5;
            let right = cell_box.x + cell_box.w;
            assert!(
                end <= right + 0.15,
                "l'image dépasse la colonne ({end} > {right})"
            );
        }
    }

    #[test]
    fn pdf_skewed_table_keeps_wide_glyphs_inside_the_narrow_column() {
        let wide = "W".repeat(40);
        let mut cells = vec![wide; 50];
        cells[0] = "‰🎉".into();
        let row = format!("|{}|", cells.join("|"));
        let pdf = generate(&Request {
            name: "Biais".into(),
            format: "pdf".into(),
            content: format!("{row}\n{}\n{row}\n", md_cells("---", 50)),
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
        let boxes = pdf_stroke_boxes(&pdf.bytes);
        let narrow = boxes.iter().map(|b| b.w).fold(f32::MAX, f32::min);
        assert!(narrow < 3.0, "la colonne étroite mesure {narrow} pt");
        let runs = pdf_text_runs(&pdf.bytes);
        let images = pdf_images(&pdf.bytes);
        assert!(
            runs.iter().any(|run| run.text.contains('‰')),
            "le pour-mille doit être dessiné"
        );
        assert!(images.len() >= 2, "les deux emoji doivent être dessinés");
        assert_ink_inside(&boxes, &runs, &images);
        assert_eq!(
            pdf_shown(&pdf.bytes).chars().filter(|c| *c == '‰').count(),
            2
        );
    }

    #[test]
    fn pdf_fitting_table_keeps_ten_point_type_and_four_point_inset() {
        let pdf = generate(&Request {
            name: "Court".into(),
            format: "pdf".into(),
            content: "| Nom | Coût |\n| --- | --- |\n| Thé | 12 |\n".into(),
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
        let runs = pdf_text_runs(&pdf.bytes);
        let boxes = pdf_stroke_boxes(&pdf.bytes);
        assert!(runs.len() >= 4, "les quatre cellules doivent être lisibles");
        for run in &runs {
            assert!(
                (run.size - 10.0).abs() < 0.011,
                "taille {} pour {}",
                run.size,
                run.text
            );
            let Some(cell_box) = box_containing(&boxes, run.x, run.y) else {
                panic!("texte hors cellule : {}", run.text);
            };
            assert!(
                (run.x - (cell_box.x + 4.0)).abs() < 0.05,
                "retrait {} pour {}",
                run.x - cell_box.x,
                run.text
            );
        }
    }

    #[test]
    fn pptx_table_rows_stay_inside_the_slide() {
        let mut content = String::from("Titre\n| A | B |\n| --- | --- |\n");
        for i in 0..17 {
            content.push_str(&format!("| r{i} | x |\n"));
        }
        let deck = generate(&Request {
            name: "Diapo".into(),
            format: "pptx".into(),
            content,
        })
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(deck.pages >= 2, "les rangs en trop changent de diapositive");
        let mut seen = vec![false; 17];
        for n in 1..=deck.pages {
            let xml = office_part(&deck.bytes, &format!("ppt/slides/slide{n}.xml"));
            assert!(xml.contains("<a:tbl>"), "diapositive {n}");
            let bottoms = shape_bottoms(&xml);
            assert!(!bottoms.is_empty(), "{xml}");
            for bottom in bottoms {
                assert!(
                    bottom <= 6_858_000,
                    "cadre à {bottom} EMU, au-delà de 6 858 000"
                );
            }
            for (i, flag) in seen.iter_mut().enumerate() {
                if xml.contains(&format!(">r{i}<")) {
                    *flag = true;
                }
            }
        }
        assert!(seen.iter().all(|flag| *flag), "rangs perdus : {seen:?}");
    }

    #[test]
    fn pdf_table_cell_expands_an_internal_tab() {
        let pdf = generate(&Request {
            name: "Tab".into(),
            format: "pdf".into(),
            content: "| Nom |\n| --- |\n| a\tb |\n".into(),
        })
        .unwrap_or_else(|e| panic!("une tabulation dans une cellule doit passer : {e:?}"));
        let shown = pdf_shown(&pdf.bytes);
        assert!(
            shown.contains("a    b"),
            "la tabulation doit valoir quatre espaces : {shown}"
        );
    }

    #[test]
    fn all_formats_are_readable_with_french_accents() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        documents::migrate(&conn).unwrap();
        for format in ["pdf", "docx", "pptx", "md", "csv", "xlsx", "svg"] {
            let content = match format {
                "csv" | "xlsx" => "[[\"Été\",\"Coût\"],[\"Cœur\",\"12 €\"]]",
                "svg" => "{\"nodes\":[\"Été\",\"Cœur\"],\"edges\":[[0,1]]}",
                _ => "Été à Québec — Cœur, coût : 12 €",
            };
            let result = generate(&Request {
                name: "Résumé".into(),
                format: format.into(),
                content: content.into(),
            })
            .unwrap();
            if format == "svg" {
                assert!(result.preview.contains("Cœur"));
            } else if format == "pdf" && !cfg!(target_os = "macos") {
                // Text extraction uses PDFKit. Off macOS, still prove a real PDF was produced.
                assert!(result.bytes.starts_with(b"%PDF-"), "{format}");
                assert!(
                    result
                        .bytes
                        .windows(b") Tj T*".len())
                        .any(|window| window == b") Tj T*"),
                    "{format}"
                );
            } else {
                let doc = documents::prepare(&conn, &result.name, &result.bytes)
                    .unwrap_or_else(|e| panic!("{format}: {e:?}"));
                assert!(doc.text.contains("Été"), "{format}: {}", doc.text);
                assert!(doc.text.contains("Cœur"), "{format}: {}", doc.text);
            }
        }
    }
    #[test]
    fn limits_and_errors_are_explicit() {
        for (format, content) in [
            ("pdf", "😀"),
            ("svg", "<script/>"),
            ("xlsx", "pas un tableau"),
            ("pptx", "---"),
            ("exe", "test"),
        ] {
            assert!(generate(&Request {
                name: "test".into(),
                format: format.into(),
                content: content.into()
            })
            .is_err());
        }
        assert!(generate(&Request {
            name: "../test".into(),
            format: "md".into(),
            content: "test".into()
        })
        .is_err());
        let bytes = csv(&[vec!["=CMD()".into(), "Été, \"oui\"".into()]]);
        assert!(String::from_utf8(bytes).unwrap().contains("'="));
        for (format, content) in [
            ("xlsx", r#"[["\u0000"]]"#),
            ("svg", r#"{"nodes":["\u0000"],"edges":[]}"#),
        ] {
            assert!(generate(&Request {
                name: "Invalide".into(),
                format: format.into(),
                content: content.into()
            })
            .is_err());
        }
        let decomposed = generate(&Request {
            name: "Accent".into(),
            format: "pdf".into(),
            content: "E\u{301}te\u{301} — coût : 1\u{202f}250 €".into(),
        })
        .unwrap();
        assert!(decomposed.bytes.starts_with(b"%PDF-"));
        // PDFKit is what proves the composed accent round-trips as readable text.
        #[cfg(target_os = "macos")]
        {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            documents::migrate(&conn).unwrap();
            assert!(
                documents::prepare(&conn, &decomposed.name, &decomposed.bytes)
                    .unwrap()
                    .text
                    .contains("Été")
            );
        }
    }
    fn well_formed(svg: &str) {
        let mut reader = quick_xml::Reader::from_str(svg);
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("SVG invalide : {e}\n{svg}"),
            }
        }
    }
    #[test]
    fn mermaid_flowcharts_render_shapes_labels_and_cycles() {
        let svg = diagram(
            "```mermaid\nflowchart TD;\n  %% commentaire\n  A[Début à Québec] --> B{Coût < 12 € ?}\n  B -->|oui| C(Été)\n  B -- non --> D[\"Cœur & âme\"]\n  D -.-> A; C --- D\n  style A fill:#f00\n  click A \"https://example.com\"\n```",
        )
        .unwrap();
        well_formed(&svg);
        for text in [
            "Début à Québec",
            "Coût &lt; 12 € ?",
            "Été",
            "Cœur &amp; âme",
            ">oui<",
            ">non<",
        ] {
            assert!(svg.contains(text), "{text}: {svg}");
        }
        assert!(svg.contains("<polygon"), "losange pour {{}}");
        assert!(svg.contains("stroke-dasharray"), "lien pointillé -.->");
        assert!(!svg.contains("example.com") && !svg.contains("fill:#f00"));
        // Five links: one is the D -.-> A back edge, one the undirected C --- D.
        assert_eq!(
            svg.matches("stroke=\"#64748b\" stroke-width=\"2\"").count(),
            5
        );
        assert_eq!(svg.matches("marker-end").count(), 4);
        let lr = diagram("graph LR\nA-->B-->C").unwrap();
        well_formed(&lr);
        let width: f64 = lr
            .split("viewBox=\"0 0 ")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(width > 600.0, "LR place les nœuds côte à côte : {lr}");
        let json = diagram("{\"nodes\":[\"Départ\",\"Arrivée\"],\"edges\":[[0,1],[1,0]]}").unwrap();
        well_formed(&json);
        for bad in [
            "sequenceDiagram\nA->>B: x",
            "flowchart XY\nA-->B",
            "flowchart TD\nA[ouvert --> B",
        ] {
            let Err(Error::InvalidInput(message)) = diagram(bad) else {
                panic!("{bad}")
            };
            assert!(message.contains("Mermaid"), "{message}");
        }
        let Err(Error::InvalidInput(message)) = diagram("graph TD\nA --> B\nC ~~> D") else {
            panic!()
        };
        assert!(
            message.contains("Ligne Mermaid non comprise : « C ~~> D »"),
            "{message}"
        );
        let long = format!("flowchart TD\nA[{}] --> B", "x".repeat(41));
        assert!(diagram(&long).is_err());
    }
    #[test]
    fn docx_reflows_paragraphs_and_formats_headings_and_bullets() {
        let result = generate(&Request {
            name: "Bilan".into(),
            format: "docx".into(),
            content: format!("# Bilan d’été\n- Première idée\n{}", "mot ".repeat(60)),
        })
        .unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&result.bytes)).unwrap();
        let mut document = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/document.xml").unwrap(),
            &mut document,
        )
        .unwrap();
        assert_eq!(
            document.matches("<w:p>").count(),
            3,
            "une ligne source, un paragraphe"
        );
        assert!(document.contains(
            "<w:b/><w:sz w:val=\"36\"/></w:rPr><w:t xml:space=\"preserve\">Bilan d’été<"
        ));
        assert!(document.contains("w:hanging") && document.contains("•\u{a0}Première idée"));
        assert!(!document.contains("# Bilan") && !document.contains("Courier"));
        assert!(result.preview.contains("font-weight=\"bold\">Bilan d’été<"));
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        documents::migrate(&conn).unwrap();
        assert!(documents::prepare(&conn, &result.name, &result.bytes)
            .unwrap()
            .text
            .contains("Bilan d’été"));
    }
    fn inside(r: &Rect, p: Point) -> bool {
        (p.0 - r.x).abs() < r.w / 2.0 - 2.0 && (p.1 - r.y).abs() < r.h / 2.0 - 2.0
    }
    /// No link passes under a node it does not join; no label covers a node or another label.
    fn assert_clear(source: &str) -> Layout {
        let graph = mermaid(source).unwrap();
        well_formed(&render(&graph));
        let layout = layout(&graph);
        for (edge, routed) in graph.edges.iter().zip(&layout.edges) {
            for (v, node) in layout.nodes.iter().enumerate() {
                if v == edge.from || v == edge.to {
                    continue;
                }
                for segment in &routed.segments {
                    for step in 0..=40 {
                        let p = segment.at(step as f64 / 40.0);
                        assert!(
                            !inside(node, p),
                            "{source}: lien {}→{} sous le nœud {v} en {p:?}",
                            edge.from,
                            edge.to
                        );
                    }
                }
            }
        }
        let labels: Vec<Rect> = layout.edges.iter().filter_map(|e| e.label).collect();
        for (i, label) in labels.iter().enumerate() {
            assert!(
                !layout.nodes.iter().any(|n| n.overlaps(label)),
                "{source}: étiquette {i} sur un nœud"
            );
            assert!(
                !labels[i + 1..].iter().any(|o| o.overlaps(label)),
                "{source}: étiquettes superposées"
            );
        }
        layout
    }
    #[test]
    fn mermaid_links_never_cross_or_hide_nodes() {
        // Review case: B→D and A→D skip ranks; the label used to hide « Traitement ».
        assert_clear("flowchart TD\n  A[Demande reçue] --> B{Complète ?}\n  B -->|oui| C[Traitement]\n  C --> D[Réponse envoyée]\n  B -->|non, rejet direct| D\n  A --> D");
        assert_clear("flowchart TD\nA-->B-->C\nA-->|non| C");
        assert_clear("graph LR\nA[Préparation]-->B-->C-->D\nA-->|raccourci très long ici| D\nD-.->|reprise| A");
        assert_clear("flowchart TD\nA-->B-->C-->D-->E\nE-->A\nD-->B\nA-->E\nC-->C\nB-->D");
        assert_clear("flowchart TD\nA & B --> C & D\nA --> D\nD --> A\nB -->|x| D");
        let svg = diagram("flowchart TD\nA-->B-->C\nA-->|non| C").unwrap();
        assert!(svg.contains(">B<") && svg.contains(">non<"));
    }
    #[test]
    fn mermaid_accepts_common_syntax_variants() {
        let graph = mermaid("graph TD; A[Début] --> B[Fin]").unwrap();
        assert_eq!((graph.nodes.len(), graph.edges.len()), (2, 1));
        let graph = mermaid("flowchart LR\nA --> B & C\nA ---> D\nD ==> E\nE === F\nF -.- G\nG -..-> H\nH == fort ==> I").unwrap();
        let links: Vec<_> = graph
            .edges
            .iter()
            .map(|e| (e.from, e.to, e.arrow, e.dashed, e.label.clone()))
            .collect();
        assert_eq!(
            links,
            vec![
                (0, 1, true, false, None),
                (0, 2, true, false, None),
                (0, 3, true, false, None),
                (3, 4, true, false, None),
                (4, 5, false, false, None),
                (5, 6, false, true, None),
                (6, 7, true, true, None),
                (7, 8, true, false, Some("fort".into())),
            ]
        );
        let svg =
            diagram("flowchart TD\nB[Mise à jour<br/>du dossier] --> C[Fin<br>du<br />cycle]")
                .unwrap();
        well_formed(&svg);
        assert!(
            !svg.contains("br")
                && svg.contains(">Mise à jour<")
                && svg.contains(">du dossier<")
                && svg.contains(">cycle<")
        );
        assert!(
            diagram("flowchart TD\nA[a<br>b<br>c<br>d] --> B").is_err(),
            "4 lignes"
        );
        let many = format!(
            "flowchart TD\n{} --> {}",
            (0..10)
                .map(|i| format!("A{i}"))
                .collect::<Vec<_>>()
                .join(" & "),
            (0..10)
                .map(|i| format!("B{i}"))
                .collect::<Vec<_>>()
                .join(" & ")
        );
        assert!(
            diagram(&many).is_err(),
            "100 liens refusés sans tout générer"
        );
    }
    #[test]
    fn previews_fit_short_documents_and_render_spreadsheet_cells() {
        for format in ["csv", "xlsx", "md", "docx", "pdf"] {
            let result = generate(&Request {
                name: "Aperçu".into(),
                format: format.into(),
                content: if matches!(format, "csv" | "xlsx") {
                    r#"[["Nom","Coût"],["Élodie","1250"],["Cœur & été","-12.50"]]"#.into()
                } else {
                    "# Bilan d’été\nÉlodie : 1250 €".into()
                },
            })
            .unwrap();
            well_formed(&result.preview);
            let size = result
                .preview
                .split("viewBox=\"0 0 ")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap();
            let height: usize = size.split_whitespace().nth(1).unwrap().parse().unwrap();
            if format == "pdf" {
                assert_eq!(height, 842);
            } else {
                assert!(height < 400, "{format}: {height}");
            }
            if matches!(format, "csv" | "xlsx") {
                assert_eq!(result.preview.matches("<rect").count(), 7);
                assert!(
                    result.preview.contains("font-weight=\"bold\"")
                        && result.preview.contains("Cœur &amp; été")
                );
            }
        }
    }
    #[test]
    fn layout_text_csv_and_xlsx_keep_data_intact() {
        assert_eq!(
            wrap("Poste      Montant\n\tLoyer    1 250 €", 70),
            vec!["Poste      Montant", "    Loyer    1 250 €"]
        );
        assert_eq!(wrap("aaaa bbbb cccc", 9), vec!["aaaa bbbb", "cccc"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        let Err(Error::InvalidInput(message)) = generate(&Request {
            name: "Étapes".into(),
            format: "pdf".into(),
            content: "Étape 1 → étape 2".into(),
        }) else {
            panic!()
        };
        assert!(message.contains("« → » (U+2192)"), "{message}");
        let text = String::from_utf8(csv(&[vec![
            "-12,50".into(),
            "+300 €".into(),
            "-2+3".into(),
            "=1+1".into(),
            "@SUM(A1)".into(),
            "-".into(),
            "Été".into(),
        ]]))
        .unwrap();
        assert_eq!(
            text,
            "\u{feff}\"-12,50\",\"+300 €\",\"'-2+3\",\"'=1+1\",\"'@SUM(A1)\",\"'-\",\"Été\"\r\n"
        );
        for (cell, number) in [
            ("1250", true),
            ("-12.5", true),
            ("0.75", true),
            ("007", false),
            ("1.", false),
            ("1 250", false),
            ("=1", false),
            ("1e5", false),
            ("1234567890123456", false),
        ] {
            assert_eq!(xlsx_number(cell), number, "{cell}");
        }
        let book = xlsx(&[vec!["Coût".into(), "1250".into(), "007".into()]]).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(&book)).unwrap();
        let mut sheet = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("xl/worksheets/sheet1.xml").unwrap(),
            &mut sheet,
        )
        .unwrap();
        assert!(
            sheet.contains("<c r=\"B1\"><v>1250</v></c>") && sheet.contains(">007</t>"),
            "{sheet}"
        );
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        documents::migrate(&conn).unwrap();
        let read = documents::prepare(&conn, "Budget.xlsx", &book).unwrap();
        assert!(
            read.text.contains("1250") && read.text.contains("Coût"),
            "{}",
            read.text
        );
    }
}
