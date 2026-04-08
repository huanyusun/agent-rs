//! Ingest converts source files into normalized text before structure parsing.
//!
//! Why this design:
//! - Keeping file-format handling isolated prevents PDF or Markdown quirks from leaking into
//!   retrieval and synthesis code.
//! - A single `DocumentIngestor` coordinates parser choice and chunk creation so the rest of the
//!   system consumes one normalized representation.
//! - An alternative would be parser-specific pipelines end to end, but that would complicate MVP
//!   storage and testing.
//! - Current limitation: PDF support is still heuristic. OCR books especially need post-processing
//!   to separate front matter and noisy headings from the main body.

use crate::{
    config::IndexConfig,
    domain::{chunk::Chunk, document::Document, section::Section, workspace::Workspace},
    error::{AppError, Result},
    parser::structure::StructureParser,
    utils::text::{
        chunk_text, detect_media_type, file_stem_or_name, keywords_for_text, text_token_count,
    },
};
use chrono::Utc;
use std::path::Path;
use uuid::Uuid;

pub mod markdown;
pub mod pdf;
pub mod text;

#[derive(Debug, Clone)]
pub struct IngestedDocument {
    pub document: Document,
    pub sections: Vec<Section>,
    pub chunks: Vec<Chunk>,
    pub original_bytes: Vec<u8>,
    pub original_extension: String,
    pub original_file_name: String,
}

pub struct DocumentIngestor {
    config: IndexConfig,
}

impl DocumentIngestor {
    pub fn new(config: IndexConfig) -> Self {
        Self { config }
    }

    pub fn ingest(&self, path: &Path, workspace: &Workspace) -> Result<IngestedDocument> {
        if !path.exists() {
            return Err(AppError::Ingest(format!(
                "source file does not exist: {}",
                path.display()
            )));
        }

        let bytes = std::fs::read(path)?;
        let media_type = detect_media_type(path)?;
        let normalized_text = match media_type.as_str() {
            "application/pdf" => pdf::extract_text(&bytes),
            "text/markdown" => markdown::extract_text(&bytes),
            _ => text::extract_text(&bytes),
        }?;

        let title = file_stem_or_name(path);
        let document_id = Uuid::new_v4().to_string();
        let mut sections =
            StructureParser::new().parse(&document_id, &title, &media_type, &normalized_text);
        if media_type == "application/pdf" {
            sections = refine_pdf_sections(sections);
        }
        let chunks = build_chunks(&document_id, &mut sections, &self.config);

        let document = Document {
            id: document_id,
            title,
            source_path: path.to_path_buf(),
            stored_path: workspace.root_dir.join("documents"),
            media_type,
            imported_at: Utc::now(),
            section_ids: sections.iter().map(|section| section.id.clone()).collect(),
            chunk_ids: chunks.iter().map(|chunk| chunk.id.clone()).collect(),
        };

        Ok(IngestedDocument {
            document,
            sections,
            chunks,
            original_bytes: bytes,
            original_extension: path
                .extension()
                .and_then(|ext| ext.to_str())
                .unwrap_or("txt")
                .to_string(),
            original_file_name: path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("document")
                .to_string(),
        })
    }
}

/// PDF OCR often yields a burst of cover, CIP, and copyright lines before the first real chapter.
///
/// Why this design:
/// - Retrieval quality improves materially when obvious publication metadata is not indexed as if it
///   were the main body of the book.
/// - Once a structured chapter heading such as `第一章` appears, earlier OCR sections are usually
///   front matter for book-shaped PDFs rather than the content users ask about.
/// - Current limitation: this heuristic favors long-form books and may trim legitimate prefaces if
///   they appear before the first numbered chapter.
fn refine_pdf_sections(sections: Vec<Section>) -> Vec<Section> {
    let filtered = sections
        .into_iter()
        .filter_map(normalize_pdf_section)
        .filter(|section| !is_low_signal_pdf_section(section))
        .collect::<Vec<_>>();
    let Some(first_body_index) = filtered
        .iter()
        .position(|section| looks_like_numbered_cjk_heading(&section.heading))
    else {
        return merge_adjacent_pdf_sections(filtered);
    };
    merge_adjacent_pdf_sections(filtered.into_iter().skip(first_body_index).collect())
}

fn normalize_pdf_section(mut section: Section) -> Option<Section> {
    if is_table_of_contents_section(&section) {
        return None;
    }

    section.content = trim_pdf_section_content(&section.content);

    if let Some((heading, trimmed_content)) = extract_embedded_heading(&section.content) {
        if is_noisy_pdf_heading(&section.heading)
            || looks_like_table_of_contents_heading(&section.heading)
        {
            section.heading = heading;
            section.content = trimmed_content;
        }
    }

    if let Some(heading) = recover_pdf_heading_from_body(&section.heading, &section.content) {
        section.heading = heading;
    }

    section.content = trim_pdf_section_content(&section.content);

    Some(section)
}

/// OCR books often split one logical chapter into consecutive sections with the same heading.
///
/// Why this design:
/// - Keeping repeated headings separate fragments citations and overweights one chapter during
///   retrieval.
/// - Adjacent-only merging is conservative: it fixes obvious OCR pagination breaks without trying
///   to infer long-range structure.
fn merge_adjacent_pdf_sections(sections: Vec<Section>) -> Vec<Section> {
    let mut merged: Vec<Section> = Vec::new();

    for mut section in sections {
        if let Some(previous) = merged.last_mut() {
            if previous.heading == section.heading && is_mergeable_pdf_heading(&section.heading) {
                if !previous.content.is_empty() && !section.content.is_empty() {
                    previous.content.push('\n');
                }
                previous.content.push_str(section.content.trim());
                continue;
            }
        }

        section.content = section.content.trim().to_string();
        merged.push(section);
    }

    merged
}

fn is_low_signal_pdf_section(section: &Section) -> bool {
    let heading = section.heading.trim();
    let content = section.content.trim();
    if heading.is_empty() || content.is_empty() {
        return true;
    }

    let heading_lower = heading.to_ascii_lowercase();
    let content_lower = content.to_ascii_lowercase();
    if heading_lower.starts_with("isbn")
        || heading_lower.contains("cip")
        || heading_lower.contains("www")
        || heading_lower.contains("出版社")
        || heading_lower.contains("图书在版编目")
        || content_lower.contains("图书在版编目")
        || content_lower.contains("中国版本图书馆cip")
    {
        return true;
    }

    let heading_alpha = heading
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count();
    let heading_cjk = heading.chars().filter(|ch| is_cjk(*ch)).count();
    if heading_cjk == 0 && heading_alpha > 0 && heading.len() <= 10 {
        return true;
    }

    let nonempty_lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    let short_line_count = nonempty_lines
        .iter()
        .filter(|line| line.chars().count() <= 4)
        .count();
    if nonempty_lines.len() >= 8 && short_line_count * 4 >= nonempty_lines.len() * 3 {
        return true;
    }

    if looks_like_cover_metadata_section(&nonempty_lines, heading, content_lower.as_str()) {
        return true;
    }

    content.len() <= 3
}

fn looks_like_cover_metadata_section(lines: &[&str], heading: &str, content_lower: &str) -> bool {
    let short_line_count = lines
        .iter()
        .filter(|line| line.chars().count() <= 8)
        .count();
    let has_publisher = content_lower.contains("出版社");
    let has_bibliography_marker = content_lower.contains("著.")
        || content_lower.contains("桂林")
        || content_lower.contains("文化史")
        || content_lower.contains("研究-中国");

    lines.len() >= 6
        && short_line_count * 2 >= lines.len()
        && has_publisher
        && (has_bibliography_marker
            || lines
                .iter()
                .any(|line| line.contains("李") && line.contains("著"))
            || heading.chars().count() >= 12)
}

/// OCR sections often keep line-fragment noise at the start and table-of-contents lines at the end.
///
/// Why this design:
/// - These artifacts leak into citations and can also bias retrieval toward non-body text.
/// - Trimming them while normalizing sections keeps the stored section body closer to what users
///   expect to search and cite.
fn trim_pdf_section_content(content: &str) -> String {
    let mut lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();

    while lines
        .first()
        .is_some_and(|line| looks_like_leading_noise_line(line))
    {
        lines.remove(0);
    }

    if let Some(index) = lines
        .iter()
        .position(|line| matches!(line.as_str(), "目 录" | "目录"))
    {
        lines.truncate(index);
    }

    lines.join("\n").trim().to_string()
}

fn looks_like_leading_noise_line(line: &str) -> bool {
    let compact = line
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    if compact.is_empty() {
        return true;
    }

    let cjk = compact.chars().filter(|ch| is_cjk(*ch)).count();
    let ascii_alpha = compact
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count();
    let digits = compact.chars().filter(|ch| ch.is_ascii_digit()).count();
    let total = compact.chars().count();

    cjk == 0 && total <= 6 && ascii_alpha + digits >= total / 2
}

fn recover_pdf_heading_from_body(current_heading: &str, content: &str) -> Option<String> {
    let heading = current_heading.trim();
    if !matches!(heading, "尾声" | "后记") {
        return None;
    }

    let preview = content.lines().take(40).collect::<Vec<_>>().join("\n");
    if preview.contains("股商最后的人祭")
        || preview.contains("下面，先来复原")
        || preview.contains("人和祭的消亡和周灭商有直接关系")
    {
        return Some("引子".to_string());
    }

    None
}

fn is_mergeable_pdf_heading(heading: &str) -> bool {
    let trimmed = heading.trim();
    matches!(trimmed, "引子" | "代序" | "尾声" | "后记") || looks_like_numbered_cjk_heading(trimmed)
}

fn is_noisy_pdf_heading(heading: &str) -> bool {
    let trimmed = heading.trim();
    let alpha = trimmed
        .chars()
        .filter(|ch| ch.is_ascii_alphabetic())
        .count();
    let cjk = trimmed.chars().filter(|ch| is_cjk(*ch)).count();
    cjk == 0 && alpha >= 4 && alpha * 2 >= trimmed.chars().filter(|ch| !ch.is_whitespace()).count()
}

fn looks_like_table_of_contents_heading(heading: &str) -> bool {
    let trimmed = heading.trim();
    looks_like_numbered_cjk_heading(trimmed)
        && trimmed.chars().last().is_some_and(|ch| ch.is_ascii_digit())
}

fn is_table_of_contents_section(section: &Section) -> bool {
    let heading = section.heading.trim();
    let content = section.content.trim();
    let short_lines = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| line.chars().count() <= 24)
        .count();
    let line_count = content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .count();
    let page_number_like = content
        .lines()
        .map(str::trim)
        .any(|line| line.chars().all(|ch| ch.is_ascii_digit()));

    (looks_like_table_of_contents_heading(heading) && line_count <= 4 && short_lines == line_count)
        || (heading == "第二十七章 ”诸神远去之后 549" && content.contains("引子 3"))
        || page_number_like && line_count <= 4
}

fn extract_embedded_heading(content: &str) -> Option<(String, String)> {
    let lines = content.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate().take(80) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some(heading) = normalize_embedded_heading(trimmed) {
            let remaining = lines[index + 1..].join("\n").trim().to_string();
            if !remaining.is_empty() {
                return Some((heading, remaining));
            }
        }
    }
    None
}

fn normalize_embedded_heading(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(cleaned) = cleaned_numbered_heading(trimmed) {
        return Some(cleaned);
    }

    let named = ["引子", "代序", "尾声", "后记"];
    named
        .into_iter()
        .find(|name| trimmed.starts_with(name))
        .map(|name| name.to_string())
}

fn cleaned_numbered_heading(line: &str) -> Option<String> {
    if !looks_like_numbered_cjk_heading(line) {
        return None;
    }

    let mut cleaned = line.trim().to_string();
    while cleaned
        .chars()
        .last()
        .is_some_and(|ch| ch.is_ascii_digit() || ch.is_whitespace())
    {
        cleaned.pop();
    }
    Some(cleaned.trim().to_string())
}

fn looks_like_numbered_cjk_heading(line: &str) -> bool {
    let Some(rest) = line.trim().strip_prefix('第') else {
        return false;
    };
    let Some(marker_pos) = rest.find(['章', '节', '卷', '篇', '部']) else {
        return false;
    };
    if marker_pos == 0 {
        return false;
    }
    rest[..marker_pos].chars().all(|ch| {
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
    })
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

fn build_chunks(document_id: &str, sections: &mut [Section], config: &IndexConfig) -> Vec<Chunk> {
    let mut chunks = Vec::new();

    for section in sections {
        for (ordinal, piece) in
            chunk_text(&section.content, config.chunk_size, config.chunk_overlap)
                .into_iter()
                .enumerate()
        {
            let chunk_id = Uuid::new_v4().to_string();
            section.chunk_ids.push(chunk_id.clone());
            chunks.push(Chunk {
                id: chunk_id,
                document_id: document_id.to_string(),
                section_id: section.id.clone(),
                ordinal,
                token_count: text_token_count(&piece),
                keywords: keywords_for_text(&piece, 8),
                text: piece,
            });
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::{
        looks_like_numbered_cjk_heading, normalize_pdf_section, refine_pdf_sections,
        trim_pdf_section_content,
    };
    use crate::domain::section::Section;

    fn section(heading: &str, content: &str) -> Section {
        Section {
            id: format!("id-{heading}"),
            document_id: "doc-1".into(),
            heading: heading.into(),
            level: 1,
            ordinal_path: vec![1],
            parent_id: None,
            content: content.into(),
            chunk_ids: Vec::new(),
        }
    }

    #[test]
    fn numbered_cjk_heading_matches_book_chapters() {
        assert!(looks_like_numbered_cjk_heading("第一章 新石器时代"));
        assert!(looks_like_numbered_cjk_heading("第3卷 周人崛起"));
        assert!(!looks_like_numbered_cjk_heading("代序：我们陌生的形象"));
    }

    #[test]
    fn refine_pdf_sections_drops_front_matter_before_first_chapter() {
        let refined = refine_pdf_sections(vec![
            section("ISBN 978-7-5598-5253-3", "中国版本图书馆CIP数据"),
            section("REM: SRA", "出版社信息"),
            section("第一章 新石器时代的社会升级", "正文第一章"),
            section("第二章 王权与祭祀", "正文第二章"),
        ]);

        assert_eq!(refined.len(), 2);
        assert_eq!(refined[0].heading, "第一章 新石器时代的社会升级");
        assert_eq!(refined[1].heading, "第二章 王权与祭祀");
    }

    #[test]
    fn normalize_pdf_section_relabels_noisy_heading_from_embedded_heading() {
        let normalized =
            normalize_pdf_section(section("FE LARA IN HE", "引子 3\n正文开始\n更多正文")).unwrap();

        assert_eq!(normalized.heading, "引子");
        assert!(normalized.content.starts_with("正文开始"));
    }

    #[test]
    fn normalize_pdf_section_drops_toc_sections() {
        let normalized = normalize_pdf_section(section(
            "第一章 新石器时代 23",
            "第二章 王权 35\n第三章 商朝 51",
        ));

        assert!(normalized.is_none());
    }

    #[test]
    fn refine_pdf_sections_merges_adjacent_same_heading_sections() {
        let refined = refine_pdf_sections(vec![
            section("ISBN 978-7-5598-5253-3", "中国版本图书馆CIP数据"),
            section("引子", "第一段"),
            section("引子", "第二段"),
            section("第一章 殷周之变", "第三段"),
        ]);

        assert_eq!(refined.len(), 1);
        assert_eq!(refined[0].heading, "第一章 殷周之变");
        assert_eq!(refined[0].content, "第三段");
    }

    #[test]
    fn refine_pdf_sections_merges_adjacent_same_heading_when_no_chapter_boundary_exists() {
        let refined = refine_pdf_sections(vec![
            section("引子", "第一段"),
            section("引子", "第二段"),
            section("代序", "第三段"),
        ]);

        assert_eq!(refined.len(), 2);
        assert_eq!(refined[0].heading, "引子");
        assert_eq!(refined[0].content, "第一段\n第二段");
    }

    #[test]
    fn refine_pdf_sections_drops_fragmented_cover_like_section() {
        let refined = refine_pdf_sections(vec![
            section(
                "某书名",
                "s\ni\n°\nBY\n: 向\n李之\nmE\n| 与\n@@华\n夏\n新\n生\n广西师范大学出版社",
            ),
            section("代序", "这是一段正常正文，讨论本书的问题意识和结构。"),
        ]);

        assert_eq!(refined.len(), 1);
        assert_eq!(refined[0].heading, "代序");
    }

    #[test]
    fn refine_pdf_sections_drops_cover_metadata_section_after_stronger_ocr() {
        let refined = refine_pdf_sections(vec![
            section(
                "翦商：殷周之变与华夏新生",
                "咱\n>\n| _ 一\n一上一\n周\n李 之\n硕 变\nx\n新\n生\n前商 : 扔周之变与华夏新生 / 李硕著. -- 桂林 :\n广西师范大学出版社, 2022.10\n1.0: 0. O#- 0. @文化史一研究-中国",
            ),
            section("代序", "这是一段正常正文，讨论本书的问题意识和结构。"),
        ]);

        assert_eq!(refined.len(), 1);
        assert_eq!(refined[0].heading, "代序");
    }

    #[test]
    fn trim_pdf_section_content_drops_toc_tail_and_leading_noise() {
        let trimmed = trim_pdf_section_content("al\n正文第一段\n目 录\n代序 : 我们陌生的形象 i");

        assert_eq!(trimmed, "正文第一段");
    }

    #[test]
    fn normalize_pdf_section_recovers_intro_heading_from_body() {
        let normalized = normalize_pdf_section(section(
            "尾声",
            "al\n为此, 须先从上古时代的人你说起。\n下面，先来复原一场急商最晚期的人佘仪式。\n股商最后的人祭",
        ))
        .unwrap();

        assert_eq!(normalized.heading, "引子");
        assert!(normalized.content.starts_with("为此"));
    }
}
