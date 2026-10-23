use crate::domain::{Doc, Kind};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn classify(filename: &str, text: &str) -> Kind {
    let name = filename.to_lowercase();
    let head = text.chars().take(1200).collect::<String>().to_lowercase();
    let csv_like = name.ends_with(".csv") || !text.contains("Subject:");
    if csv_like && head.starts_with("role,rate") {
        return Kind::RateSheet;
    }
    if csv_like && head.starts_with("invoice,amount,request") {
        return Kind::PriorBilling;
    }
    if csv_like && head.starts_with("line_id,document,excerpt") {
        return Kind::EvidenceExtract;
    }
    if name.contains("receipt") || head.starts_with("line_id,received_qty") {
        return Kind::DeliveryReceipt;
    }
    if csv_like && (head.starts_with("line_id,description") || name.contains("cost-export")) {
        return Kind::CostExport;
    }
    let subject_is_return = head
        .lines()
        .find(|line| line.starts_with("subject:"))
        .map(|line| line.contains("returned") || line.contains("return"))
        .unwrap_or(false);
    if name.ends_with(".eml") || text.contains("Subject:") {
        return if subject_is_return || name.contains("return-note") {
            Kind::ReturnNote
        } else {
            Kind::Directive
        };
    }
    if name.contains("ticket") || head.contains("field ticket") {
        Kind::FieldTicket
    } else if name.contains("scope") || head.starts_with("base scope") || head.contains("exclusions:") {
        Kind::Scope
    } else if name.contains("invoice") || head.starts_with("invoice") {
        Kind::Invoice
    } else {
        Kind::Other
    }
}

pub fn parse_upload(filename: &str, bytes: &[u8]) -> Result<Doc, String> {
    let name = Path::new(filename)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("upload.txt");
    let safe = sanitize_filename(name)?;
    let lower = safe.to_lowercase();
    if !(lower.ends_with(".txt")
        || lower.ends_with(".csv")
        || lower.ends_with(".eml")
        || lower.ends_with(".md")
        || lower.ends_with(".pdf"))
    {
        return Err("Upload a CSV, EML, TXT, MD, or uncompressed text PDF.".into());
    }
    if bytes.len() > 5_000_000 {
        return Err("That file is over 5 MB.".into());
    }
    let text = if lower.ends_with(".pdf") {
        extract_pdf_text(bytes)?
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|_| "The file is not valid UTF-8 text.".to_string())?
    };
    if text.trim().is_empty() {
        return Err("No text could be read from that file.".into());
    }
    let hash = sha256_hex(bytes);
    Ok(Doc {
        id: format!("doc-{}", &hash[..12]),
        filename: safe.clone(),
        kind: classify(&safe, &text),
        text,
        sha256: hash,
    })
}

fn sanitize_filename(name: &str) -> Result<String, String> {
    let cleaned: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' ))
        .take(80)
        .collect();
    if cleaned.is_empty() || cleaned.starts_with('.') {
        return Err("The file name is not usable.".into());
    }
    Ok(cleaned)
}

pub fn extract_pdf_text(bytes: &[u8]) -> Result<String, String> {
    let raw = String::from_utf8_lossy(bytes);
    if raw.contains("/FlateDecode") || raw.contains("/Filter") {
        return Err("This PDF is compressed. ExtraWork reads uncompressed text PDFs, or a TXT, CSV, or EML export.".into());
    }
    let mut out = String::new();
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '(' {
            i += 1;
            let mut literal = String::new();
            while i < chars.len() && chars[i] != ')' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    literal.push(chars[i]);
                } else {
                    literal.push(chars[i]);
                }
                i += 1;
            }
            if literal.chars().any(|c| c.is_alphanumeric()) {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&literal);
            }
        }
        i += 1;
    }
    if out.trim().is_empty() {
        return Err("No text operators were found in this PDF.".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_harbor_file() {
        let text = include_str!("../../fixtures/harbor-annex/return-note.eml");
        assert_eq!(classify("return-note.eml", text), Kind::ReturnNote);
        assert_eq!(
            classify("cost-export.csv", include_str!("../../fixtures/harbor-annex/cost-export.csv")),
            Kind::CostExport
        );
        assert_eq!(
            classify("field-ticket.txt", include_str!("../../fixtures/harbor-annex/field-ticket.txt")),
            Kind::FieldTicket
        );
        assert_eq!(
            classify(
                "delivery-receipt.csv",
                include_str!("../../fixtures/northgate-mechanical/delivery-receipt.csv")
            ),
            Kind::DeliveryReceipt
        );
    }

    #[test]
    fn rejects_a_compressed_pdf_instead_of_inventing_text() {
        let err = extract_pdf_text(b"%PDF-1.4 /Filter /FlateDecode").unwrap_err();
        assert!(err.contains("compressed"));
    }

    #[test]
    fn reads_an_uncompressed_pdf_literal() {
        let pdf = b"%PDF-1.4\n(FIELD TICKET Friday overtime)\n";
        let text = extract_pdf_text(pdf).unwrap();
        assert!(text.contains("FIELD TICKET"));
    }
}
