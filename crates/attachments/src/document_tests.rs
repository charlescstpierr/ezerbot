use super::*;
use rusqlite::{params, Connection};
use std::io::{Cursor, Write};

fn db() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn
}
fn office(parts: &[(&str, &str)]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("[Content_Types].xml", options).unwrap();
    zip.write_all(b"<Types/>").unwrap();
    for (name, text) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap().into_inner()
}
/// A real, searchable one-page PDF, with a valid xref. No test-only parser.
fn pdf() -> Vec<u8> {
    pdf_with_stream("BT /F1 18 Tf 40 100 Td (Bonjour PDF local) Tj ET")
}
fn pdf_with_stream(stream: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".into(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".into(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>".into(),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        format!("<< /Length {} >>\nstream\n{stream}\nendstream", stream.len()),
    ];
    let mut out = "%PDF-1.4\n".to_string();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.push_str(&format!("{} 0 obj\n{object}\nendobj\n", i + 1));
    }
    let xref = out.len();
    out.push_str("xref\n0 6\n0000000000 65535 f \n");
    for offset in offsets {
        out.push_str(&format!("{offset:010} 00000 n \n"));
    }
    out.push_str(&format!(
        "trailer\n<< /Size 6 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF"
    ));
    out.into_bytes()
}
#[test]
fn plain_formats_utf16_and_unicode_truncation() {
    let conn = db();
    for name in ["notes.txt", "plan.MD", "ventes.csv"] {
        let doc = stage(&conn, name, "Été,42\nMars,53".as_bytes()).unwrap();
        assert_eq!(doc.name, name);
        assert_eq!(doc.text, "Été,42\nMars,53");
        assert_eq!(doc.page_kind, "pages estimées");
        assert!(!doc.truncated);
    }
    let mut utf16 = vec![255, 254];
    for word in "Allô UTF16".encode_utf16() {
        utf16.extend(word.to_le_bytes());
    }
    assert_eq!(
        stage(&conn, "texte.txt", &utf16).unwrap().text,
        "Allô UTF16"
    );
    let doc = stage(&conn, "long.md", "é".repeat(MAX_TEXT + 10).as_bytes()).unwrap();
    assert_eq!(doc.text.chars().count(), MAX_TEXT);
    assert!(doc.truncated);
    assert!(doc.note.unwrap().contains("Texte tronqué"));
}
#[test]
fn office_docx_spreadsheet_shared_multiline_and_slide_caps() {
    let conn = db();
    let docx = office(&[("word/document.xml", "<w:document xmlns:w='urn:w'><w:body><w:p><w:r><w:t>Bonjour &amp; merci</w:t></w:r></w:p><w:p><w:r><w:t>Deuxième paragraphe</w:t></w:r></w:p></w:body></w:document>")]);
    let doc = stage(&conn, "rapport.docx", &docx).unwrap();
    assert!(doc.text.contains("Bonjour & merci\nDeuxième paragraphe"));
    let xlsx = office(&[
        ("xl/sharedStrings.xml", "<sst><si><t>Libellé\nlong</t></si><si><r><t>Mon</t></r><r><t>tréal</t></r></si><si/></sst>"),
        ("xl/worksheets/sheet1.xml", "<worksheet><sheetData><row><c t='s'><v>0</v></c><c t='s'><v>1</v></c><c><v>42</v></c><c t='inlineStr'><is><t>Fini</t></is></c><c t='s'><v>2</v></c></row></sheetData></worksheet>"),
    ]);
    let sheet = stage(&conn, "tableau.xlsx", &xlsx).unwrap();
    assert!(
        sheet.text.contains("Libellé\nlong\tMontréal\t42\tFini\t"),
        "{}",
        sheet.text
    );
    assert_eq!(sheet.page_count, 1);
    assert_eq!(sheet.page_kind, "feuilles");
    let names: Vec<String> = (1..=MAX_PAGES + 1)
        .map(|i| format!("ppt/slides/slide{i}.xml"))
        .collect();
    let slides = office(
        &names
            .iter()
            .map(|name| {
                (
                    name.as_str(),
                    "<slide><p><t>Une diapositive</t></p></slide>",
                )
            })
            .collect::<Vec<_>>(),
    );
    let deck = stage(&conn, "présentation.pptx", &slides).unwrap();
    assert_eq!(deck.page_count, MAX_PAGES + 1);
    assert_eq!(deck.pages_read, MAX_PAGES);
    assert!(deck.truncated);
    assert!(!deck.text.contains("diapositives 51"));
    assert!(stage(
        &conn,
        "diagramme.pptx",
        &office(&[("ppt/slides/slide1.xml", "<slide><shape/></slide>")])
    )
    .unwrap_err()
    .to_string()
    .contains("Aucun texte"));
}
#[test]
#[cfg(target_os = "macos")]
fn real_macos_pdfkit_and_rtf_readers() {
    let conn = db();
    let doc = stage(&conn, "lecture.pdf", &pdf()).unwrap();
    assert!(doc.text.contains("Bonjour PDF local"), "{}", doc.text);
    assert_eq!(doc.page_count, 1);
    assert_eq!(doc.page_kind, "pages");
    assert!(stage(
        &conn,
        "sans-texte.pdf",
        &pdf_with_stream("0 0 100 100 re S")
    )
    .unwrap_err()
    .to_string()
    .contains("OCR"));
    let rtf = stage(
        &conn,
        "lettre.rtf",
        b"{\\rtf1\\ansi Bonjour \\b RTF\\b0  local}",
    )
    .unwrap();
    assert!(rtf.text.contains("Bonjour RTF local"));
}
#[test]
#[cfg(target_os = "macos")]
fn physical_pdf_page_cap_and_real_encryption() {
    let conn = db();
    let many = builtin("/usr/bin/osascript", &["-l", "JavaScript", "-e", r#"ObjC.import('Foundation'); ObjC.import('PDFKit');
const data=$.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
const result=$.PDFDocument.alloc.init;
for(let i=0;i<51;i++){const source=$.PDFDocument.alloc.initWithData(data);result.insertPageAtIndex(source.pageAtIndex(0),i);}
$.NSFileHandle.fileHandleWithStandardOutput.writeData(result.dataRepresentation);"#], &pdf()).unwrap();
    let doc = stage(&conn, "51-pages.pdf", &many).unwrap();
    assert_eq!(doc.page_count, 51);
    assert_eq!(doc.pages_read, 50);
    assert!(doc.truncated);
    assert!(doc.text.contains("[Page 50]"));
    assert!(!doc.text.contains("[Page 51]"));
    let locked = builtin("/usr/bin/osascript", &["-l", "JavaScript", "-e", r#"ObjC.import('Foundation'); ObjC.import('PDFKit');
const data=$.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
const doc=$.PDFDocument.alloc.initWithData(data); const options=$.NSMutableDictionary.alloc.init;
options.setObjectForKey($('test-password'),$.PDFDocumentUserPasswordOption);
options.setObjectForKey($('test-owner'),$.PDFDocumentOwnerPasswordOption);
$.NSFileHandle.fileHandleWithStandardOutput.writeData(doc.dataRepresentationWithOptions(options));"#], &pdf()).unwrap();
    assert!(stage(&conn, "protégé.pdf", &locked)
        .unwrap_err()
        .to_string()
        .contains("Ce PDF est chiffré"));
}

#[test]
fn invalid_encrypted_oversized_and_bombs_are_french_errors() {
    let conn = db();
    for (name, bytes, expected) in [
        ("vide.txt", vec![], "vide"),
        ("image.txt", vec![0, 1, 2], "binaires"),
        ("ancien.docx", b"\xd0\xcf\x11\xe0fake".to_vec(), "chiffré"),
        ("bad.docx", b"not a zip".to_vec(), "invalide"),
        ("bad.pdf", b"fake".to_vec(), "PDF est invalide"),
        (
            "bad.xlsx",
            office(&[("word/document.xml", "<doc><t>Faux</t></doc>")]),
            "aucune",
        ),
        (
            "bad.docx",
            office(&[("word/document.xml", "<doc><t>non fermé")]),
            "incomplet",
        ),
        (
            "bad.docx",
            office(&[(
                "word/document.xml",
                "<!DOCTYPE foo SYSTEM 'https://example.invalid'><doc><t>texte</t></doc>",
            )]),
            "DTD",
        ),
        (
            "bad.docx",
            office(&[("word/document.xml", &"a".repeat(16 * 1024 * 1024 + 1))]),
            "décompression",
        ),
        ("trop.txt", vec![b'a'; MAX_BYTES + 1], "20 Mio"),
        ("programme.exe", b"text".to_vec(), "Format non accepté"),
        ("../notes.txt", b"text".to_vec(), "nom"),
    ] {
        let error = stage(&conn, name, &bytes).unwrap_err().to_string();
        assert!(error.contains(expected), "{name}: {error}");
    }
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM attachments_documents", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
}
#[test]
fn concurrent_staging_deduplicates_durable_extraction_and_links() {
    use std::sync::{Arc, Barrier};
    let mut nonce = [0; 16];
    getrandom::getrandom(&mut nonce).unwrap();
    let directory = std::env::temp_dir().join(format!(
        "documents-concurrent-{}",
        crate::staging::digest(&nonce)
    ));
    std::fs::create_dir(&directory).unwrap();
    let path = directory.join("workspace.sqlite");
    let conn = Connection::open(&path).unwrap();
    migrate(&conn).unwrap();
    conn.pragma_update(None, "journal_mode", "WAL").unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let bytes = Arc::new(
        "Texte partagé : Montréal, budget de 42 dollars.\n"
            .repeat(1000)
            .into_bytes(),
    );
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let (path, barrier, bytes) = (path.clone(), barrier.clone(), bytes.clone());
            std::thread::spawn(move || {
                let mut conn = Connection::open(path).unwrap();
                conn.busy_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                barrier.wait();
                let doc = stage(&conn, "concurrent.md", &bytes).unwrap();
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .unwrap();
                link(
                    &tx,
                    "same-conversation",
                    "same-message",
                    std::slice::from_ref(&doc.id),
                )
                .unwrap();
                tx.commit().unwrap();
                doc
            })
        })
        .collect();
    let documents: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(documents.iter().all(|d| d == &documents[0]));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM attachments_documents", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        for_message(&conn, "same-conversation", "same-message").unwrap(),
        vec![documents[0].clone()]
    );
    drop(conn);
    let reopened = Connection::open(&path).unwrap();
    assert_eq!(
        get(&reopened, &documents[0].id).unwrap(),
        Some(documents[0].clone())
    );
    drop(reopened);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn corrupt_zip_checksum_is_a_french_error_and_empty_shared_strings_are_valid() {
    let conn = db();
    let mut corrupt = office(&[("word/document.xml", "<doc><p><t>Bonjour</t></p></doc>")]);
    let central = corrupt
        .windows(4)
        .enumerate()
        .filter(|(_, s)| *s == b"PK\x01\x02")
        .map(|(i, _)| i)
        .next_back()
        .unwrap();
    corrupt[central + 16] ^= 0xff;
    let error = stage(&conn, "corrompu.docx", &corrupt)
        .unwrap_err()
        .to_string();
    assert!(error.contains("Document Office corrompu"), "{error}");
    let xlsx = office(&[
        ("xl/sharedStrings.xml", "<sst/>"),
        (
            "xl/worksheets/sheet1.xml",
            "<worksheet><row><c r='C2'><v>42</v></c></row></worksheet>",
        ),
    ]);
    assert!(stage(&conn, "nombres.xlsx", &xlsx)
        .unwrap()
        .text
        .contains("[C2] 42"));
    let blank = office(&[(
        "xl/worksheets/sheet1.xml",
        "<worksheet><row><c r='C2'></c></row></worksheet>",
    )]);
    assert!(stage(&conn, "vide.xlsx", &blank)
        .unwrap_err()
        .to_string()
        .contains("Aucun texte"));
}

#[test]
fn extracted_text_and_links_persist_without_parsing_again() {
    let path =
        std::env::temp_dir().join(format!("documents-persist-{}.sqlite", std::process::id()));
    let conn = Connection::open(&path).unwrap();
    migrate(&conn).unwrap();
    let mut doc = stage(&conn, "persist.txt", b"Texte original").unwrap();
    link(&conn, "a", "same", &[doc.id.clone(), doc.id.clone()]).unwrap();
    assert!(for_message(&conn, "b", "same").unwrap().is_empty());
    assert!(link(&conn, "a", "invalid", &[doc.id.clone(), "missing".into()]).is_err());
    assert!(for_message(&conn, "a", "invalid").unwrap().is_empty());
    doc.text = "Extraction persistée, pas reparsée".into();
    conn.execute(
        "UPDATE attachments_documents SET record=?1 WHERE id=?2",
        params![serde_json::to_string(&doc).unwrap(), doc.id],
    )
    .unwrap();
    drop(conn);
    let reopened = Connection::open(&path).unwrap();
    migrate(&reopened).unwrap();
    assert_eq!(
        stage(&reopened, "persist.txt", b"Texte original").unwrap(),
        doc
    );
    assert_eq!(for_message(&reopened, "a", "same").unwrap(), vec![doc]);
    drop(reopened);
    std::fs::remove_file(path).unwrap();
}
