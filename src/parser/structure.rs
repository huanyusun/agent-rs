use crate::domain::section::Section;
use uuid::Uuid;

/// Structure parsing rebuilds document hierarchy from normalized text.
///
/// Why this design:
/// - Research answers need section-aware citations, so ingest cannot stop at raw text.
/// - The parser uses heading heuristics because they work across markdown, text, and coarse PDF
///   extraction without requiring a different parser pipeline per format.
/// - An alternative would be dedicated AST builders for each format, but that is unnecessary for
///   the first runnable harness.
/// - Current limitation: non-heading-rich documents still fall back to synthetic sections, and the
///   heuristic heading detector is intentionally simple rather than layout-aware.
pub struct StructureParser;

impl StructureParser {
    pub fn new() -> Self {
        Self
    }

    pub fn parse(
        &self,
        document_id: &str,
        title: &str,
        media_type: &str,
        text: &str,
    ) -> Vec<Section> {
        let mut sections = Vec::new();
        let mut current_heading = title.to_string();
        let mut current_level = 1usize;
        let mut current_lines = Vec::new();
        let mut counters: Vec<usize> = vec![0];

        for line in text.lines() {
            if let Some((heading, level)) = detect_heading(line, media_type) {
                if let Some(section) = maybe_build_section(
                    document_id,
                    &current_heading,
                    current_level,
                    &counters,
                    current_lines.join("\n"),
                ) {
                    sections.push(section);
                    current_lines.clear();
                }
                current_heading = heading;
                current_level = level;
                update_counters(&mut counters, level);
                continue;
            }
            current_lines.push(line.to_string());
        }

        if current_lines.is_empty() && sections.is_empty() {
            current_lines.push(text.to_string());
        }

        if let Some(section) = maybe_build_section(
            document_id,
            &current_heading,
            current_level,
            &counters,
            current_lines.join("\n"),
        ) {
            sections.push(section);
        }

        if sections.is_empty() {
            sections.push(build_section(document_id, title, 1, &[1], text.to_string()));
        }

        attach_parent_ids(sections)
    }
}

impl Default for StructureParser {
    fn default() -> Self {
        Self::new()
    }
}

fn detect_heading(line: &str, media_type: &str) -> Option<(String, usize)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    if media_type == "text/markdown" && trimmed.starts_with('#') {
        let level = trimmed.chars().take_while(|ch| *ch == '#').count().max(1);
        let heading = trimmed[level..].trim();
        if !heading.is_empty() {
            return Some((heading.to_string(), level));
        }
    }

    // Chinese history and non-fiction books often use chapter labels such as "第一章" or
    // "第七节". Recognizing them keeps PDF-derived text from collapsing into one giant section.
    if let Some(level) = detect_cjk_heading_level(trimmed) {
        return Some((trimmed.to_string(), level));
    }

    let words: Vec<&str> = trimmed.split_whitespace().collect();
    let uppercase_word_count = words
        .iter()
        .filter(|word| word.chars().filter(|ch| ch.is_ascii_alphabetic()).count() >= 3)
        .count();
    let has_long_uppercase_word = words
        .iter()
        .any(|word| word.chars().filter(|ch| ch.is_ascii_alphabetic()).count() >= 4);
    if words.len() <= 10
        && trimmed.chars().all(|ch| {
            ch.is_ascii_uppercase()
                || ch.is_ascii_whitespace()
                || ch.is_ascii_digit()
                || matches!(ch, '.' | ':' | '-')
        })
        && (trimmed.len() >= 12 || (uppercase_word_count >= 2 && has_long_uppercase_word))
    {
        return Some((trimmed.to_string(), 2));
    }

    None
}

fn detect_cjk_heading_level(line: &str) -> Option<usize> {
    let rest = line.strip_prefix('第')?;
    let marker_pos = rest.find(['章', '节', '卷', '篇', '部'])?;
    if marker_pos == 0 {
        return None;
    }

    let numeral = &rest[..marker_pos];
    if numeral.chars().all(|ch| {
        ch.is_ascii_digit()
            || matches!(
                ch,
                '一' | '二'
                    | '三'
                    | '四'
                    | '五'
                    | '六'
                    | '七'
                    | '八'
                    | '九'
                    | '十'
                    | '百'
                    | '千'
                    | '零'
                    | '〇'
                    | '两'
            )
    }) {
        let marker = rest[marker_pos..].chars().next()?;
        let level = match marker {
            '部' | '卷' => 1,
            '篇' | '章' => 2,
            '节' => 3,
            _ => 2,
        };
        return Some(level);
    }

    None
}

fn update_counters(counters: &mut Vec<usize>, level: usize) {
    if counters.len() < level {
        counters.resize(level, 0);
    }
    counters.truncate(level);
    if let Some(last) = counters.last_mut() {
        *last += 1;
    }
}

fn build_section(
    document_id: &str,
    heading: &str,
    level: usize,
    counters: &[usize],
    content: String,
) -> Section {
    Section {
        id: Uuid::new_v4().to_string(),
        document_id: document_id.to_string(),
        heading: heading.to_string(),
        level,
        ordinal_path: counters.to_vec(),
        parent_id: None,
        content: content.trim().to_string(),
        chunk_ids: Vec::new(),
    }
}

fn maybe_build_section(
    document_id: &str,
    heading: &str,
    level: usize,
    counters: &[usize],
    content: String,
) -> Option<Section> {
    if content.trim().is_empty() {
        None
    } else {
        Some(build_section(
            document_id,
            heading,
            level,
            counters,
            content,
        ))
    }
}

fn attach_parent_ids(mut sections: Vec<Section>) -> Vec<Section> {
    for index in 0..sections.len() {
        let parent_id = if sections[index].level <= 1 {
            None
        } else {
            sections[..index]
                .iter()
                .rev()
                .find(|candidate| candidate.level < sections[index].level)
                .map(|candidate| candidate.id.clone())
        };
        sections[index].parent_id = parent_id;
    }
    sections
}

#[cfg(test)]
mod tests {
    use super::{detect_cjk_heading_level, detect_heading};

    #[test]
    fn detects_cjk_chapter_headings() {
        assert_eq!(detect_cjk_heading_level("第一章 殷周之变"), Some(2));
        assert_eq!(detect_cjk_heading_level("第七节 祭祀"), Some(3));
        assert_eq!(detect_cjk_heading_level("第3卷 新秩序"), Some(1));
    }

    #[test]
    fn detect_heading_uses_cjk_heuristic() {
        let heading = detect_heading("第一章 殷周之变", "application/pdf");
        assert_eq!(heading, Some(("第一章 殷周之变".to_string(), 2)));
    }

    #[test]
    fn detect_heading_rejects_short_ascii_ocr_noise() {
        assert_eq!(detect_heading("BY", "application/pdf"), None);
        assert_eq!(detect_heading("IV. OK203", "application/pdf"), None);
        assert_eq!(detect_heading("REM: SRA", "application/pdf"), None);
    }
}
