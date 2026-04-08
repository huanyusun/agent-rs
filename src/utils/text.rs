use crate::error::{AppError, Result};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

pub fn detect_media_type(path: &Path) -> Result<String> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .ok_or_else(|| AppError::Ingest(format!("file has no extension: {}", path.display())))?;
    let media_type = match ext.to_ascii_lowercase().as_str() {
        "md" | "markdown" => "text/markdown",
        "txt" => "text/plain",
        "pdf" => "application/pdf",
        other => {
            return Err(AppError::Ingest(format!(
                "unsupported file extension: {}",
                other
            )))
        }
    };
    Ok(media_type.to_string())
}

pub fn file_stem_or_name(path: &Path) -> String {
    path.file_stem()
        .or_else(|| path.file_name())
        .and_then(|value| value.to_str())
        .unwrap_or("document")
        .to_string()
}

pub fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![];
    }

    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < chars.len() {
        let end = (start + chunk_size).min(chars.len());
        let piece = chars[start..end]
            .iter()
            .collect::<String>()
            .trim()
            .to_string();
        if !piece.is_empty() {
            chunks.push(piece);
        }
        if end == chars.len() {
            break;
        }
        start = end.saturating_sub(overlap);
    }
    chunks
}

pub fn text_token_count(text: &str) -> usize {
    tokenize(text).len()
}

/// Tokenization feeds both keyword overlap and the lightweight hashed embedding.
///
/// Why this design:
/// - The original MVP only supported ASCII word boundaries, which made Chinese queries and OCR text
///   effectively invisible to retrieval scoring.
/// - We now keep the ASCII path for Latin text and add overlapping CJK bigrams so mixed-language
///   corpora still work without introducing a full tokenizer dependency.
/// - Current limitation: CJK bigrams are a pragmatic retrieval heuristic, not a linguistic
///   segmenter, so downstream ranking is still approximate.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut ascii = String::new();
    let mut cjk_run = Vec::new();

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            if !cjk_run.is_empty() {
                flush_cjk_run(&mut cjk_run, &mut tokens);
            }
            ascii.push(ch.to_ascii_lowercase());
            continue;
        }

        if !ascii.is_empty() {
            flush_ascii_term(&mut ascii, &mut tokens);
        }

        if is_cjk(ch) {
            cjk_run.push(ch);
        } else if !cjk_run.is_empty() {
            flush_cjk_run(&mut cjk_run, &mut tokens);
        }
    }

    if !ascii.is_empty() {
        flush_ascii_term(&mut ascii, &mut tokens);
    }
    if !cjk_run.is_empty() {
        flush_cjk_run(&mut cjk_run, &mut tokens);
    }

    tokens
}

pub fn keywords_for_text(text: &str, limit: usize) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for token in tokenize(text) {
        *counts.entry(token).or_insert(0) += 1;
    }
    let mut pairs = counts.into_iter().collect::<Vec<_>>();
    pairs.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    pairs.into_iter().take(limit).map(|pair| pair.0).collect()
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

static STOP_WORDS: std::sync::LazyLock<HashSet<&'static str>> = std::sync::LazyLock::new(|| {
    [
        "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "in", "into", "is", "it",
        "of", "on", "or", "that", "the", "this", "to", "with", "what", "which",
    ]
    .into_iter()
    .collect()
});

fn flush_ascii_term(term: &mut String, tokens: &mut Vec<String>) {
    if term.len() >= 2 && !STOP_WORDS.contains(term.as_str()) {
        tokens.push(term.clone());
    }
    term.clear();
}

fn flush_cjk_run(run: &mut Vec<char>, tokens: &mut Vec<String>) {
    if run.len() == 1 {
        tokens.push(run[0].to_string());
    } else {
        for window in run.windows(2) {
            tokens.push(window.iter().collect());
        }
    }
    run.clear();
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

#[cfg(test)]
mod tests {
    use super::tokenize;

    #[test]
    fn tokenize_keeps_ascii_terms() {
        assert_eq!(
            tokenize("Research harness for local docs"),
            vec!["research", "harness", "local", "docs"]
        );
    }

    #[test]
    fn tokenize_emits_cjk_bigrams_for_chinese_text() {
        assert_eq!(tokenize("殷周之变"), vec!["殷周", "周之", "之变"]);
    }

    #[test]
    fn tokenize_supports_mixed_cjk_and_ascii() {
        assert_eq!(
            tokenize("OCR 识别殷周"),
            vec!["ocr", "识别", "别殷", "殷周"]
        );
    }
}
