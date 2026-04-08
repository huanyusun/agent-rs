use crate::error::{AppError, Result};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

/// PDF ingest prefers true text extraction and uses byte scanning only as a last resort.
///
/// Why this design:
/// - The previous implementation treated the whole PDF as a lossy byte string, which kept the MVP
///   runnable but produced `%PDF`, stream, and image payload noise for many real books.
/// - A dedicated extractor gives `ask` a fair chance to retrieve readable chunk text before we
///   spend time improving ranking or prompts.
/// - We still keep a constrained fallback so text-based PDFs that partly fail extraction can return
///   something inspectable instead of crashing the ingest pipeline outright.
/// - Current limitation: scanned PDFs without embedded text need OCR. This module can use local
///   `pdftoppm` and `tesseract` when they are installed, but the default OCR path intentionally
///   limits how many pages it processes so large books stay tractable in an MVP CLI.
pub fn extract_text(bytes: &[u8]) -> Result<String> {
    if let Ok(extracted) = pdf_extract::extract_text_from_mem(bytes) {
        let cleaned = clean_extracted_text(&extracted);
        if is_readable_enough(&cleaned) {
            return Ok(cleaned);
        }
    }

    let fallback = clean_extracted_text(&fallback_extract_text(bytes));
    if is_readable_enough(&fallback) {
        return Ok(fallback);
    }

    if looks_like_image_only_pdf(bytes) {
        if let Some(ocr_text) = ocr_extract_text(bytes)? {
            if is_readable_enough(&ocr_text) {
                return Ok(ocr_text);
            }
        }
    }

    let detail = if looks_like_image_only_pdf(bytes) {
        "pdf extraction produced no readable text; the file looks image-only, and OCR either is unavailable or did not recover enough text. Install `pdftoppm` plus Tesseract with a language pack such as `chi_sim`, or raise `RESEARCH_HARNESS_PDF_OCR_MAX_PAGES` for longer OCR runs."
    } else {
        "pdf extraction produced no readable text; use a text-based PDF or add OCR for scanned documents"
    };
    Err(AppError::Ingest(detail.into()))
}

fn clean_extracted_text(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| !is_layout_noise_line(line))
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|line| denoise_body_line(&line))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_readable_enough(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 80 {
        return false;
    }

    let lowercase = trimmed.to_ascii_lowercase();
    let pdf_marker_hits = [
        "%pdf",
        "endobj",
        "stream",
        "endstream",
        "jpxdecode",
        "flatedecode",
        "xref",
        "startxref",
    ]
    .into_iter()
    .filter(|marker| lowercase.contains(marker))
    .count();
    if pdf_marker_hits >= 2 {
        return false;
    }

    let visible = trimmed.chars().filter(|ch| !ch.is_control()).count();
    if visible == 0 {
        return false;
    }

    let readable = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || ch.is_whitespace() || is_cjk(*ch))
        .count();
    (readable as f32 / visible as f32) >= 0.6
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x30000..=0x3134F
    )
}

/// OCR for scanned books often emits page headers, figure captions, and short garbage lines.
///
/// Why this design:
/// - These lines are repeated across pages and distort section recovery more than they help
///   retrieval.
/// - Filtering at the text-normalization layer keeps later section parsing and chunking simpler.
/// - The rules stay conservative: lines with enough CJK body text are preserved even if they
///   contain digits.
fn is_layout_noise_line(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return true;
    }

    let compact = trimmed
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let cjk = compact.chars().filter(|ch| is_cjk(*ch)).count();
    let ascii_alpha = compact
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count();
    let digits = compact.chars().filter(|ch| ch.is_ascii_digit()).count();
    let total = compact.chars().count();

    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("图书在版编目")
        || lower.contains("cip")
        || lower.contains("isbn")
        || lower.contains("www.")
    {
        return true;
    }

    if trimmed.contains("平面图") || trimmed.contains("示意图") || trimmed.contains("剖面图")
    {
        return true;
    }

    if trimmed.contains("股周之变与华夏新生") {
        return true;
    }

    if looks_like_running_header_line(trimmed) {
        return true;
    }

    if cjk == 0 && ascii_alpha >= 3 && ascii_alpha * 2 >= total && total <= 24 {
        return true;
    }

    if cjk == 0 && ascii_alpha >= 8 && ascii_alpha * 2 >= total {
        return true;
    }

    if cjk <= 2 && digits >= 1 && total <= 12 {
        return true;
    }

    if total <= 10 && cjk == 0 && digits == total {
        return true;
    }

    false
}

fn looks_like_running_header_line(line: &str) -> bool {
    let compact = line
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    let cjk = compact.chars().filter(|ch| is_cjk(*ch)).count();
    let digits = compact.chars().filter(|ch| ch.is_ascii_digit()).count();
    let total = compact.chars().count();
    if cjk < 4 || total > 32 || has_sentence_punctuation(line) {
        return false;
    }

    let first = line.split_whitespace().next().unwrap_or_default();
    let last = line.split_whitespace().last().unwrap_or_default();
    (digits >= 1 && first.chars().all(|ch| ch.is_ascii_digit()))
        || is_page_marker_token(last)
        || line.contains("殷周之变与华夏新生")
}

fn has_sentence_punctuation(line: &str) -> bool {
    line.chars()
        .any(|ch| matches!(ch, '。' | '！' | '？' | '；' | '：'))
}

fn is_page_marker_token(token: &str) -> bool {
    let trimmed = token.trim_matches(|ch: char| !ch.is_alphanumeric());
    !trimmed.is_empty()
        && trimmed.len() <= 4
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_digit() || matches!(ch, 'i' | 'v' | 'x' | 'I' | 'V' | 'X'))
}

/// OCR body lines often keep short Latin fragments that are not part of the real Chinese prose.
///
/// Why this design:
/// - These fragments degrade readability and keyword extraction while rarely helping retrieval.
/// - The rule only activates for lines that already look like Chinese body text, which keeps
///   mixed-language content from losing meaningful Latin terms.
fn denoise_body_line(line: &str) -> String {
    let cjk = line.chars().filter(|ch| is_cjk(*ch)).count();
    let ascii_alpha = line.chars().filter(|ch| ch.is_ascii_alphabetic()).count();
    if cjk < 4 || ascii_alpha == 0 || ascii_alpha >= cjk * 2 {
        return line.to_string();
    }

    let cleaned = line
        .split_whitespace()
        .filter(|token| !is_ascii_noise_token(token))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string();
    strip_ascii_garbage_prefix(&cleaned)
}

fn is_ascii_noise_token(token: &str) -> bool {
    let compact = token.trim_matches(|ch: char| !ch.is_alphanumeric());
    if compact.is_empty() {
        return false;
    }

    let ascii_alpha = compact
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count();
    let digits = compact.chars().filter(|ch| ch.is_ascii_digit()).count();
    let cjk = compact.chars().filter(|ch| is_cjk(*ch)).count();
    if cjk > 0 {
        return false;
    }

    let len = compact.chars().count();
    if ascii_alpha >= 2 && digits == 0 && len <= 6 {
        return true;
    }

    if ascii_alpha == 1 && digits == 0 && len <= 2 {
        return true;
    }

    if ascii_alpha == 1 && digits >= 1 && len <= 4 {
        return true;
    }

    ascii_alpha >= 2 && digits >= 1 && len <= 8
}

fn strip_ascii_garbage_prefix(line: &str) -> String {
    let mut index = 0usize;
    let chars = line.chars().collect::<Vec<_>>();
    while index < chars.len() {
        let ch = chars[index];
        if is_cjk(ch) || ch.is_ascii_digit() {
            break;
        }
        if matches!(ch, '“' | '”' | '《' | '》') {
            break;
        }
        index += 1;
    }

    let trimmed = chars[index..].iter().collect::<String>().trim().to_string();
    if trimmed.chars().filter(|ch| is_cjk(*ch)).count() >= 4 {
        trimmed
    } else {
        line.to_string()
    }
}

fn fallback_extract_text(bytes: &[u8]) -> String {
    let raw = String::from_utf8_lossy(bytes);
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in raw.chars() {
        if ch.is_ascii_graphic() || ch.is_ascii_whitespace() {
            current.push(ch);
        } else if current.len() >= 4 {
            tokens.push(current.clone());
            current.clear();
        } else {
            current.clear();
        }
    }
    if current.len() >= 4 {
        tokens.push(current);
    }

    tokens.join(" ").replace("(cid:", " ").replace("\\n", "\n")
}

fn looks_like_image_only_pdf(bytes: &[u8]) -> bool {
    let raw = String::from_utf8_lossy(bytes).to_ascii_lowercase();
    let image_markers = ["/subtype/image", "jpxdecode", "dctdecode", "ccittfaxdecode"]
        .into_iter()
        .filter(|marker| raw.contains(marker))
        .count();
    image_markers >= 2
}

fn ocr_extract_text(bytes: &[u8]) -> Result<Option<String>> {
    if !command_exists("pdftoppm") || !command_exists("tesseract") {
        return Ok(None);
    }

    let temp_root = env::temp_dir().join(format!("research-harness-ocr-{}", Uuid::new_v4()));
    fs::create_dir_all(&temp_root)?;
    let pdf_path = temp_root.join("source.pdf");
    fs::write(&pdf_path, bytes)?;

    let page_prefix = temp_root.join("page");
    let max_pages = ocr_max_pages();
    let raster_status = Command::new("pdftoppm")
        .arg("-png")
        .arg("-gray")
        .arg("-r")
        .arg("300")
        .arg("-f")
        .arg("1")
        .arg("-l")
        .arg(max_pages.to_string())
        .arg(&pdf_path)
        .arg(&page_prefix)
        .status();
    if !matches!(raster_status, Ok(status) if status.success()) {
        cleanup_temp_dir(&temp_root);
        return Ok(None);
    }

    let lang = tesseract_lang().unwrap_or_else(|| "eng".to_string());
    let mut images = fs::read_dir(&temp_root)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("png"))
        .collect::<Vec<_>>();
    images.sort();

    let mut pages = Vec::new();
    for image in images {
        if let Some(text) = run_tesseract_best(&image, &lang)? {
            let cleaned = clean_extracted_text(&text);
            if !cleaned.is_empty() {
                pages.push(cleaned);
            }
        }
    }

    cleanup_temp_dir(&temp_root);
    if pages.is_empty() {
        Ok(None)
    } else {
        Ok(Some(pages.join("\n\n")))
    }
}

/// Scanned books vary across pages, so one fixed Tesseract page mode is brittle.
///
/// Why this design:
/// - `--psm 6` is strong for dense single blocks, while some pages OCR better with a looser
///   segmentation mode.
/// - Trying two bounded configs and picking the more readable output improves quality without
///   pulling in a heavier OCR stack.
fn run_tesseract_best(image: &Path, lang: &str) -> Result<Option<String>> {
    let mut best_text = None;
    let mut best_score = 0.0f32;

    for psm in ["6", "4"] {
        if let Some(text) = run_tesseract(image, lang, psm)? {
            let score = ocr_text_score(&text);
            if best_text.is_none() || score > best_score {
                best_score = score;
                best_text = Some(text);
            }
        }
    }

    Ok(best_text)
}

fn run_tesseract(image: &Path, lang: &str, psm: &str) -> Result<Option<String>> {
    let output = Command::new("tesseract")
        .arg(image)
        .arg("stdout")
        .arg("-l")
        .arg(lang)
        .arg("--oem")
        .arg("1")
        .arg("--psm")
        .arg(psm)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            Ok(Some(String::from_utf8_lossy(&output.stdout).into()))
        }
        Ok(_) => Ok(None),
        Err(error) => Err(AppError::Ingest(format!(
            "failed to run tesseract: {error}"
        ))),
    }
}

fn ocr_text_score(text: &str) -> f32 {
    let cleaned = clean_extracted_text(text);
    if cleaned.is_empty() {
        return 0.0;
    }

    let cjk = cleaned.chars().filter(|ch| is_cjk(*ch)).count() as f32;
    let ascii_alpha = cleaned
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count() as f32;
    let punctuation = cleaned
        .chars()
        .filter(|ch| ch.is_ascii_punctuation())
        .count() as f32;
    let length = cleaned.chars().count() as f32;

    cjk * 2.0 + length * 0.05 - ascii_alpha * 0.8 - punctuation * 0.2
}

fn tesseract_lang() -> Option<String> {
    let output = Command::new("tesseract")
        .arg("--list-langs")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let langs = String::from_utf8_lossy(&output.stdout);
    if langs.lines().any(|line| line.trim() == "chi_sim") {
        Some("chi_sim+eng".to_string())
    } else if langs.lines().any(|line| line.trim() == "eng") {
        Some("eng".to_string())
    } else {
        None
    }
}

fn ocr_max_pages() -> usize {
    env::var("RESEARCH_HARNESS_PDF_OCR_MAX_PAGES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(24)
}

fn command_exists(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn cleanup_temp_dir(path: &PathBuf) {
    let _ = fs::remove_dir_all(path);
}

#[cfg(test)]
mod tests {
    use super::{
        clean_extracted_text, denoise_body_line, is_ascii_noise_token, is_layout_noise_line,
        is_readable_enough, looks_like_image_only_pdf, ocr_max_pages, ocr_text_score,
        strip_ascii_garbage_prefix,
    };

    #[test]
    fn clean_extracted_text_keeps_line_boundaries_but_normalizes_spacing() {
        let cleaned = clean_extracted_text("  第一章   变革 \n\n  殷周  之变   ");
        assert_eq!(cleaned, "第一章 变革\n殷周 之变");
    }

    #[test]
    fn clean_extracted_text_drops_layout_noise_lines() {
        let cleaned = clean_extracted_text(
            "iv ii: 股周之变与华夏新生\n后网 H10 第二层平面图\n第一章 殷周之变\n正文保留",
        );
        assert_eq!(cleaned, "第一章 殷周之变\n正文保留");
    }

    #[test]
    fn clean_extracted_text_drops_running_headers_and_long_ascii_noise_lines() {
        let cleaned = clean_extracted_text(
            "10 殷周之变与华夏新生\nSHRMA ERB NARA. (RIC) SRBCM, ALBA\n正文保留",
        );
        assert_eq!(cleaned, "正文保留");
    }

    #[test]
    fn readable_text_accepts_cjk_content() {
        let text = "翦商讨论殷周之变与华夏新生。".repeat(20);
        assert!(is_readable_enough(&text));
    }

    #[test]
    fn readable_text_rejects_binary_like_noise() {
        let text = "%PDF-1.6 JPXDecode stream endobj qGPu >=h a'@TR;".repeat(20);
        assert!(!is_readable_enough(&text));
    }

    #[test]
    fn image_only_pdf_detection_flags_image_stream_markers() {
        let bytes = br#"%PDF-1.6 /Subtype/Image /Filter/JPXDecode /Subtype/Image"#;
        assert!(looks_like_image_only_pdf(bytes));
    }

    #[test]
    fn ocr_max_pages_uses_default_when_env_missing() {
        std::env::remove_var("RESEARCH_HARNESS_PDF_OCR_MAX_PAGES");
        assert_eq!(ocr_max_pages(), 24);
    }

    #[test]
    fn layout_noise_rule_keeps_real_body_text() {
        assert!(!is_layout_noise_line(
            "周人对商朝人祭制度进行了系统性的改造。"
        ));
        assert!(is_layout_noise_line("BATH T 0"));
        assert!(is_layout_noise_line("后网 H10 第二层平面图"));
    }

    #[test]
    fn denoise_body_line_removes_short_ascii_fragments_in_cjk_prose() {
        let cleaned =
            denoise_body_line("作者认为人祭的消亡和周灭商有直接关系 HEART 19 人 引发了华夏的新生");
        assert_eq!(
            cleaned,
            "作者认为人祭的消亡和周灭商有直接关系 19 人 引发了华夏的新生"
        );
    }

    #[test]
    fn denoise_body_line_keeps_latin_when_line_is_not_cjk_body() {
        let cleaned = denoise_body_line("NotebookLM research harness OCR");
        assert_eq!(cleaned, "NotebookLM research harness OCR");
    }

    #[test]
    fn denoise_body_line_strips_ascii_prefix_from_cjk_line() {
        let cleaned = denoise_body_line("BFE ET ATR ASR i. AE, 就是杀人向鬼神献祭。");
        assert_eq!(cleaned, "就是杀人向鬼神献祭。");
    }

    #[test]
    fn ascii_noise_token_rule_targets_short_ocr_garbage() {
        assert!(is_ascii_noise_token("HEART"));
        assert!(is_ascii_noise_token("H10"));
        assert!(!is_ascii_noise_token("NotebookLM"));
        assert!(!is_ascii_noise_token("2022"));
    }

    #[test]
    fn ocr_text_score_prefers_cjk_heavier_output() {
        assert!(ocr_text_score("周灭商与华夏新生") > ocr_text_score("RUB ar HEART H10 BY"));
    }

    #[test]
    fn strip_ascii_garbage_prefix_keeps_real_cjk_body() {
        let cleaned = strip_ascii_garbage_prefix("Ext (Bid RA) PARA EAE 并不符合当时的规则。");
        assert_eq!(cleaned, "并不符合当时的规则。");
    }
}
