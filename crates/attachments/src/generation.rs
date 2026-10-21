//! Local reconstruction: REA parity.md names formats, not a recovered generator.
//! Small, deterministic writers reuse the bounded ZIP dependency already used by
//! the reader. No macros, formulas, remote assets, scripts or shell execution.
use crate::documents;
use grokbot_engine::contracts::{Error, Result};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Write};
use unicode_normalization::UnicodeNormalization;
use zip::{write::SimpleFileOptions, ZipWriter};

const MAX_CONTENT: usize = 100_000;
const MAX_LINES: usize = 1800;
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
    let bytes = match format {
        "md" => request.content.as_bytes().to_vec(),
        "pdf" => pdf(&lines)?,
        "docx" => docx(&normalized)?,
        "pptx" => {
            let slides = slides(&request.content)?;
            pages = slides.len();
            preview_lines = slides[0].clone();
            pptx(&slides)?
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
    let preview = table.unwrap_or_else(|| text_preview(&preview_lines, format));
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
    // Thumbnails use the file's own family: Courier for PDF and plain text,
    // the sans-serif Office font for DOCX and PPTX.
    let family = if matches!(format, "docx" | "pptx") {
        "Arial,Helvetica,sans-serif"
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
// Standard PDF font with explicit WinAnsi encoding: all French letters, œ/Œ,
// guillemets and typographic punctuation. Unsupported glyphs fail, never vanish.
fn winansi(c: char) -> Result<u8> {
    Ok(match c {
        '\u{20}'..='\u{7e}'|'\u{a0}'..='\u{ff}' => c as u8,
        '\u{202f}'=>160,'€'=>128,'‚'=>130,'ƒ'=>131,'„'=>132,'…'=>133,'†'=>134,'‡'=>135,'ˆ'=>136,'‰'=>137,'Š'=>138,'‹'=>139,'Œ'=>140,'Ž'=>142,'‘'=>145,'’'=>146,'“'=>147,'”'=>148,'•'=>149,'–'=>150,'—'=>151,'˜'=>152,'™'=>153,'š'=>154,'›'=>155,'œ'=>156,'ž'=>158,'Ÿ'=>159,
        _=> return Err(invalid(&format!("Ce PDF contient « {c} » (U+{:04X}), non pris en charge par Courier. Remplacez-le ou utilisez DOCX (les accents français sont pris en charge en PDF).", c as u32))),
    })
}
fn pdf(lines: &[String]) -> Result<Vec<u8>> {
    let chunks: Vec<_> = lines.chunks(48).collect();
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        format!(
            "<< /Type /Pages /Count {} /Kids [{}] >>",
            chunks.len(),
            (0..chunks.len())
                .map(|i| format!("{} 0 R", 4 + i * 2))
                .collect::<Vec<_>>()
                .join(" ")
        )
        .into_bytes(),
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec(),
    ];
    for (i, chunk) in chunks.iter().enumerate() {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",5+i*2).into_bytes());
        let mut stream = b"BT /F1 12 Tf 15 TL 42 786 Td\n".to_vec();
        for line in *chunk {
            stream.push(b'(');
            for c in line.chars() {
                let b = winansi(c)?;
                if matches!(b, b'(' | b')' | b'\\') {
                    stream.push(b'\\');
                }
                stream.push(b);
            }
            stream.extend_from_slice(b") Tj T*\n");
        }
        stream.extend_from_slice(b"ET\n");
        let mut obj = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
        obj.extend(stream);
        obj.extend_from_slice(b"endstream");
        objects.push(obj);
    }
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
    Ok(out)
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
// headings and "- " bullets become formatting; other markers stay literal text.
fn docx(content: &str) -> Result<Vec<u8>> {
    let paragraphs = content
        .lines()
        .map(|line| {
            let (level, text) = heading(line);
            let bullet = level == 0
                && (line.trim_start().starts_with("- ") || line.trim_start().starts_with("* "));
            let text = if bullet { &line.trim_start()[2..] } else { text };
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
        })
        .collect::<String>();
    package(vec![("word/document.xml".into(),format!("{XML}<w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\"><w:body>{paragraphs}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/></w:sectPr></w:body></w:document>"))],vec![("word/document.xml".into(),"wordprocessingml.document.main".into())],"word/document.xml")
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
fn slides(content: &str) -> Result<Vec<Vec<String>>> {
    let mut slides = vec![String::new()];
    for line in content.lines() {
        if line.trim() == "---" {
            slides.push(String::new());
        } else {
            let last = slides.last_mut().unwrap();
            last.push_str(line);
            last.push('\n');
        }
    }
    if slides.len() > 50 || slides.iter().any(|s| s.trim().is_empty()) {
        return Err(invalid("La présentation doit contenir 1 à 50 diapositives non vides, séparées par une ligne --- ."));
    }
    slides.iter().map(|s| {
        let mut input=s.lines();
        let mut lines=wrap(input.next().unwrap_or(""), 50);
        lines.extend(wrap(&input.collect::<Vec<_>>().join("\n"), 80));
        if lines.len()>16 { Err(invalid("Une diapositive dépasse 16 lignes. Répartissez son contenu avec une ligne --- .")) } else { Ok(lines) }
    }).collect()
}
const GROUP:&str="<p:nvGrpSpPr><p:cNvPr id=\"1\" name=\"\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/><a:chOff x=\"0\" y=\"0\"/><a:chExt cx=\"0\" cy=\"0\"/></a:xfrm></p:grpSpPr>";
fn pptx(slides: &[Vec<String>]) -> Result<Vec<u8>> {
    let ns=format!("xmlns:p=\"http://schemas.openxmlformats.org/presentationml/2006/main\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"{REL}\"");
    let mut files = vec![];
    let mut types = vec![];
    let mut links = vec![(
        "rId1".to_string(),
        "slideMaster".to_string(),
        "slideMasters/slideMaster1.xml".to_string(),
    )];
    let mut ids = String::new();
    for (i, lines) in slides.iter().enumerate() {
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
        let paragraphs=lines.iter().enumerate().map(|(j,l)|format!("<a:p><a:pPr><a:lnSpc><a:spcPts val=\"2800\"/></a:lnSpc><a:spcBef><a:spcPts val=\"0\"/></a:spcBef><a:spcAft><a:spcPts val=\"0\"/></a:spcAft></a:pPr><a:r><a:rPr lang=\"fr-CA\" sz=\"{}\"><a:solidFill><a:srgbClr val=\"172033\"/></a:solidFill><a:latin typeface=\"Arial\"/></a:rPr><a:t>{}</a:t></a:r></a:p>",if j==0{2800}else{1800},xml(l))).collect::<String>();
        let path = format!("ppt/slides/slide{n}.xml");
        types.push((path.clone(), "presentationml.slide".into()));
        files.push((path,format!("{XML}<p:sld {ns}><p:cSld><p:bg><p:bgPr><a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill><a:effectLst/></p:bgPr></p:bg><p:spTree>{GROUP}<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Contenu\"/><p:cNvSpPr txBox=\"1\"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x=\"500000\" y=\"400000\"/><a:ext cx=\"11192000\" cy=\"6000000\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></p:spPr><p:txBody><a:bodyPr wrap=\"square\"/><a:lstStyle/>{paragraphs}</p:txBody></p:sp></p:spTree></p:cSld><p:clrMapOvr><a:masterClrMapping/></p:clrMapOvr></p:sld>")));
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
                // The text reader is PDFKit. Off macOS, still require a real PDF file.
                assert!(result.bytes.starts_with(b"%PDF-"), "{format}");
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
