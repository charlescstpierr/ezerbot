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
fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for line in s.lines() {
        let mut current = String::new();
        for word in line.split_whitespace() {
            if current.chars().count() + word.chars().count() + 1 > width && !current.is_empty() {
                lines.push(std::mem::take(&mut current));
            }
            for c in word.chars() {
                if current.chars().count() >= width {
                    lines.push(std::mem::take(&mut current));
                }
                current.push(c);
            }
            current.push(' ');
        }
        lines.push(current.trim_end().into());
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
            preview_lines = rows.iter().take(15).map(|r| r.join("  |  ")).collect();
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
    let preview = text_preview(&preview_lines, format);
    Ok(Generated {
        name,
        format: format.into(),
        bytes,
        preview,
        pages,
    })
}
fn text_preview(lines: &[String], format: &str) -> String {
    let slide = format == "pptx";
    let (w, h, count) = if slide {
        (960, 540, 16)
    } else {
        (595, 842, 48)
    };
    // Thumbnails use the file's own family: Courier for PDF and plain text,
    // the sans-serif Office font for DOCX and PPTX.
    let family = if matches!(format, "docx" | "pptx") {
        "Arial,Helvetica,sans-serif"
    } else {
        "Courier New,Courier,monospace"
    };
    let mut svg=format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w} {h}\"><rect width=\"{w}\" height=\"{h}\" fill=\"#fff\"/><g fill=\"#172033\" font-family=\"{family}\" font-size=\"12\">");
    let mut y = 42;
    for (i, line) in lines.iter().take(count).enumerate() {
        let (heading, text) = if format == "docx" {
            heading(line)
        } else {
            (0, line.as_str())
        };
        let (font_size, advance) = match (slide, heading) {
            (true, _) if i == 0 => (28, 28),
            (true, _) => (18, 28),
            (false, 1) => (18, 26),
            (false, 2) => (15, 22),
            (false, 3) => (13, 19),
            _ => (12, 15),
        };
        y += advance;
        // Clip long table cells explicitly in the thumbnail, not in the file.
        let visible = text
            .chars()
            .take(if slide { 80 } else { 70 })
            .collect::<String>();
        svg.push_str(&format!(
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
    svg.push_str("</g></svg>");
    svg
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
        _=> return Err(invalid("Ce PDF contient un caractère non pris en charge par Courier. Utilisez DOCX pour ce texte (les accents français sont pris en charge en PDF).")),
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
                    let value = if s.trim_start().starts_with(['=', '+', '-', '@', '\t', '\r']) {
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
    let data=rows.iter().enumerate().map(|(i,row)|format!("<row r=\"{}\">{}</row>",i+1,row.iter().enumerate().map(|(j,s)|format!("<c r=\"{}{}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",column(j),i+1,xml(s))).collect::<String>())).collect::<String>();
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
fn mermaid_error() -> Error {
    invalid("Diagramme Mermaid non pris en charge : commencez par « flowchart TD » ou « flowchart LR », puis des liens comme A[Départ] --> B{Choix} -->|oui| C(Arrivée).")
}
// Subset of Mermaid flowcharts: nodes, shapes, links and link labels. Styling,
// click handlers and subgraph boxes are ignored; nothing in the input is executed.
fn mermaid(content: &str) -> Result<Graph> {
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%") && !l.starts_with("```"));
    let mut header = lines
        .next()
        .ok_or_else(mermaid_error)?
        .trim_end_matches(';')
        .split_whitespace();
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
    for statement in lines.flat_map(|l| l.split(';')).map(str::trim) {
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
        let mut rest = statement;
        let mut previous: Option<(usize, Link)> = None;
        loop {
            let (node, after) = mermaid_node(rest, &mut ids, &mut graph)?;
            if let Some((from, (label, dashed, arrow))) = previous.take() {
                graph.edges.push(Edge {
                    from,
                    to: node,
                    label,
                    dashed,
                    arrow,
                });
            }
            let after = after.trim_start();
            if after.is_empty() {
                break;
            }
            let (link, after) = mermaid_link(after).ok_or_else(mermaid_error)?;
            previous = Some((node, link));
            rest = after.trim_start();
        }
    }
    Ok(graph)
}
fn mermaid_node<'a>(
    s: &'a str,
    ids: &mut Vec<String>,
    graph: &mut Graph,
) -> Result<(usize, &'a str)> {
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    let (id, rest) = s.split_at(end);
    if id.is_empty() {
        return Err(mermaid_error());
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
            let close_at = inner.find(close).ok_or_else(mermaid_error)?;
            let label = inner[..close_at].trim();
            let label = label
                .strip_prefix('"')
                .and_then(|l| l.strip_suffix('"'))
                .unwrap_or(label);
            definition = Some((label.to_string(), shape, &inner[close_at + close.len()..]));
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
    Ok(match definition {
        Some((label, shape, rest)) => {
            graph.nodes[index] = Node { label, shape };
            (index, rest)
        }
        None => (index, rest),
    })
}
fn mermaid_link(s: &str) -> Option<(Link, &str)> {
    for (op, dashed, arrow) in [
        ("-.->", true, true),
        ("-->", false, true),
        ("==>", false, true),
        ("---", false, false),
    ] {
        if let Some(rest) = s.strip_prefix(op) {
            let trimmed = rest.trim_start();
            if let Some(inner) = trimmed.strip_prefix('|') {
                let end = inner.find('|')?;
                let label = inner[..end].trim();
                return Some((
                    ((!label.is_empty()).then(|| label.into()), dashed, arrow),
                    &inner[end + 1..],
                ));
            }
            return Some(((None, dashed, arrow), rest));
        }
    }
    // Mermaid's inline label form: A -- texte --> B
    let inner = s.strip_prefix("--")?;
    let end = inner.find("-->")?;
    let label = inner[..end].trim();
    Some((
        ((!label.is_empty()).then(|| label.into()), false, true),
        &inner[end + 3..],
    ))
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
        || graph.nodes.len() > 20
        || graph.edges.len() > 40
        || graph.nodes.iter().any(|n| {
            n.label.trim().is_empty() || n.label.chars().count() > 40 || !valid_text(&n.label)
        })
        || graph
            .edges
            .iter()
            .filter_map(|e| e.label.as_ref())
            .any(|l| l.chars().count() > 30 || !valid_text(l))
    {
        return Err(invalid("Diagramme invalide : 1 à 20 nœuds (40 caractères chacun), 40 liens maximum (étiquettes de 30 caractères)."));
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
// Layered layout: cycles are broken by DFS back edges, ranks follow the longest
// path, and back edges loop around the side so they never cross a node.
fn render(graph: &Graph) -> String {
    let n = graph.nodes.len();
    let mut state = vec![0; n];
    let mut back = vec![false; graph.edges.len()];
    for v in 0..n {
        if state[v] == 0 {
            mark_back_edges(graph, v, &mut state, &mut back);
        }
    }
    let mut rank = vec![0usize; n];
    for _ in 0..n {
        for (e, _) in graph.edges.iter().zip(&back).filter(|(_, b)| !**b) {
            rank[e.to] = rank[e.to].max(rank[e.from] + 1);
        }
    }
    let size = |node: &Node| {
        let w = (node.label.chars().count() as f64 * 10.0 + 40.0).clamp(120.0, 440.0);
        if node.shape == Shape::Diamond {
            (w + 60.0, 76.0)
        } else {
            (w, 56.0)
        }
    };
    let ranks = rank.iter().max().unwrap() + 1;
    let mut layers = vec![Vec::new(); ranks];
    for v in 0..n {
        layers[rank[v]].push(v);
    }
    let widest = graph.nodes.iter().map(|n| size(n).0).fold(0.0, f64::max);
    let widest_layer = layers.iter().map(Vec::len).max().unwrap() as f64;
    let (cell, step) = if graph.horizontal {
        (110.0, widest + 110.0)
    } else {
        (widest + 50.0, 140.0)
    };
    let (inner_w, inner_h) = if graph.horizontal {
        (ranks as f64 * step - 110.0, widest_layer * cell)
    } else {
        (widest_layer * cell, ranks as f64 * step - 64.0)
    };
    let (w, h) = (inner_w + 160.0, inner_h + 120.0);
    let mut center = vec![(0.0, 0.0); n];
    for (r, layer) in layers.iter().enumerate() {
        let offset = (widest_layer - layer.len() as f64) * cell / 2.0;
        for (j, &v) in layer.iter().enumerate() {
            let across = 40.0 + offset + (j as f64 + 0.5) * cell;
            center[v] = if graph.horizontal {
                (40.0 + widest / 2.0 + r as f64 * step, across + 20.0)
            } else {
                (across, 40.0 + 38.0 + r as f64 * step)
            };
        }
    }
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {w:.0} {h:.0}\" font-family=\"Helvetica,Arial,sans-serif\"><rect width=\"{w:.0}\" height=\"{h:.0}\" fill=\"#fff\"/><defs><marker id=\"arrow\" markerWidth=\"10\" markerHeight=\"10\" refX=\"9\" refY=\"5\" orient=\"auto\"><path d=\"M0 0 L10 5 L0 10Z\" fill=\"#64748b\"/></marker></defs>");
    let mut labels = String::new();
    let mut loops = 0.0;
    for (e, &is_back) in graph.edges.iter().zip(&back) {
        let ((sx, sy), (tx, ty)) = (center[e.from], center[e.to]);
        let ((sw, sh), (tw, th)) = (size(&graph.nodes[e.from]), size(&graph.nodes[e.to]));
        let (path, mid) = if is_back {
            loops += 1.0;
            if graph.horizontal {
                let (a, b) = ((sx, sy + sh / 2.0), (tx, ty + th / 2.0));
                let y = a.1.max(b.1) + 30.0 + 12.0 * loops;
                (
                    format!(
                        "M{:.0} {:.0} C{:.0} {y:.0} {:.0} {y:.0} {:.0} {:.0}",
                        a.0, a.1, a.0, b.0, b.0, b.1
                    ),
                    ((a.0 + b.0) / 2.0, y - 8.0),
                )
            } else {
                let (a, b) = ((sx + sw / 2.0, sy), (tx + tw / 2.0, ty));
                let x = a.0.max(b.0) + 34.0 + 12.0 * loops;
                (
                    format!(
                        "M{:.0} {:.0} C{x:.0} {:.0} {x:.0} {:.0} {:.0} {:.0}",
                        a.0, a.1, a.1, b.1, b.0, b.1
                    ),
                    (x - 8.0, (a.1 + b.1) / 2.0),
                )
            }
        } else if graph.horizontal {
            let (a, b) = ((sx + sw / 2.0, sy), (tx - tw / 2.0, ty));
            let m = (a.0 + b.0) / 2.0;
            (
                format!(
                    "M{:.0} {:.0} C{m:.0} {:.0} {m:.0} {:.0} {:.0} {:.0}",
                    a.0, a.1, a.1, b.1, b.0, b.1
                ),
                (m, (a.1 + b.1) / 2.0),
            )
        } else {
            let (a, b) = ((sx, sy + sh / 2.0), (tx, ty - th / 2.0));
            let m = (a.1 + b.1) / 2.0;
            (
                format!(
                    "M{:.0} {:.0} C{:.0} {m:.0} {:.0} {m:.0} {:.0} {:.0}",
                    a.0, a.1, a.0, b.0, b.0, b.1
                ),
                ((a.0 + b.0) / 2.0, m),
            )
        };
        svg.push_str(&format!(
            "<path d=\"{path}\" fill=\"none\" stroke=\"#64748b\" stroke-width=\"2\"{}{}/>",
            if e.dashed {
                " stroke-dasharray=\"6 5\""
            } else {
                ""
            },
            if e.arrow {
                " marker-end=\"url(#arrow)\""
            } else {
                ""
            }
        ));
        if let Some(label) = &e.label {
            let lw = label.chars().count() as f64 * 7.5 + 14.0;
            labels.push_str(&format!("<rect x=\"{:.0}\" y=\"{:.0}\" width=\"{lw:.0}\" height=\"22\" rx=\"4\" fill=\"#fff\" stroke=\"#cbd5e1\"/><text x=\"{:.0}\" y=\"{:.0}\" text-anchor=\"middle\" font-size=\"13\" fill=\"#334155\">{}</text>", mid.0 - lw / 2.0, mid.1 - 11.0, mid.0, mid.1 + 4.5, xml(label)));
        }
    }
    for (node, &(x, y)) in graph.nodes.iter().zip(&center) {
        let (nw, nh) = size(node);
        let (l, t) = (x - nw / 2.0, y - nh / 2.0);
        svg.push_str(&match node.shape {
            Shape::Diamond => format!("<polygon points=\"{x:.0},{t:.0} {:.0},{y:.0} {x:.0},{:.0} {l:.0},{y:.0}\" fill=\"#fef3c7\" stroke=\"#d97706\" stroke-width=\"1.5\"/>", l + nw, t + nh),
            shape => format!("<rect x=\"{l:.0}\" y=\"{t:.0}\" width=\"{nw:.0}\" height=\"{nh:.0}\" rx=\"{}\" fill=\"#eff6ff\" stroke=\"#2563eb\" stroke-width=\"1.5\"/>", if shape == Shape::Round { 28 } else { 8 }),
        });
        svg.push_str(&format!(
            "<text x=\"{x:.0}\" y=\"{:.0}\" text-anchor=\"middle\" font-size=\"16\" fill=\"#172033\">{}</text>",
            y + 5.5,
            xml(&node.label)
        ));
    }
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
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        documents::migrate(&conn).unwrap();
        assert!(
            documents::prepare(&conn, &decomposed.name, &decomposed.bytes)
                .unwrap()
                .text
                .contains("Été")
        );
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
            "flowchart TD\nA & B --> C",
            "flowchart TD\nA[ouvert --> B",
        ] {
            let Err(Error::InvalidInput(message)) = diagram(bad) else {
                panic!("{bad}")
            };
            assert!(message.contains("Mermaid"), "{message}");
        }
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
}
