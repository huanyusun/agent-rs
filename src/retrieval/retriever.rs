use crate::{
    domain::{chunk::Chunk, citation::Citation, document::Document, section::Section},
    error::{AppError, Result},
    index::simple_index::{SimpleIndex, SimpleIndexData},
    storage::workspace_store::WorkspaceCorpus,
};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct RetrievalRequest {
    pub query: String,
    pub top_k: usize,
}

#[derive(Debug, Clone)]
pub struct RetrievalHit {
    pub chunk: Chunk,
    pub section: RetrievalSection,
    pub document: RetrievalDocument,
    pub score: f32,
    pub citation: Citation,
}

/// RetrievalSection carries only the section fields needed after ranking.
///
/// Why this design:
/// - `ask` cites section headings, but it does not need the full `section.content` once chunks have
///   already been selected.
/// - Keeping retrieval hits lightweight avoids cloning the same giant section text for every
///   matching chunk, which is especially important for PDFs that collapse into a single section.
/// - An alternative would be to keep cloning full `Section` values for convenience, but that turns
///   a chunk-level query path into a large hidden memory copy.
#[derive(Debug, Clone)]
pub struct RetrievalSection {
    pub id: String,
    pub heading: String,
    pub level: usize,
    pub ordinal_path: Vec<usize>,
    pub parent_id: Option<String>,
}

impl From<&Section> for RetrievalSection {
    fn from(section: &Section) -> Self {
        Self {
            id: section.id.clone(),
            heading: section.heading.clone(),
            level: section.level,
            ordinal_path: section.ordinal_path.clone(),
            parent_id: section.parent_id.clone(),
        }
    }
}

/// RetrievalDocument carries only the document fields needed for prompt evidence and citations.
///
/// Why this design:
/// - The synthesis step needs a stable title for prompt formatting and user-facing provenance.
/// - The full `Document` metadata is still stored on disk, but repeating paths and timestamps in
///   every retrieval hit adds noise without helping answer generation.
#[derive(Debug, Clone)]
pub struct RetrievalDocument {
    pub id: String,
    pub title: String,
    pub media_type: String,
}

impl From<&Document> for RetrievalDocument {
    fn from(document: &Document) -> Self {
        Self {
            id: document.id.clone(),
            title: document.title.clone(),
            media_type: document.media_type.clone(),
        }
    }
}

/// Retriever ranks chunks and expands them back into human-readable evidence packets.
///
/// Why this design:
/// - Retrieval should stay deterministic and inspectable, so this layer only scores chunks and
///   resolves references to documents and sections.
/// - The expansion step is important because synthesis should not know how to join raw ids back to
///   provenance objects, but the expanded hit should still stay small enough for large-PDF queries.
/// - An alternative would be to store denormalized section and document data directly in the index,
///   but that would duplicate metadata and make updates more fragile.
/// - Current limitation: ranking is purely local and does not rerank with a semantic model.
pub struct Retriever {
    index: SimpleIndex,
    default_top_k: usize,
}

impl Retriever {
    pub fn new(index: SimpleIndex, default_top_k: usize) -> Self {
        Self {
            index,
            default_top_k,
        }
    }

    pub fn rebuild_index(&self, current: SimpleIndexData, chunks: &[Chunk]) -> SimpleIndexData {
        self.index.merge(current, chunks)
    }

    pub fn retrieve(
        &self,
        request: &RetrievalRequest,
        index: &SimpleIndexData,
        corpus: &WorkspaceCorpus,
    ) -> Result<Vec<RetrievalHit>> {
        let top_k = request.top_k.max(1).min(self.default_top_k.max(1));
        let chunk_by_id = corpus
            .chunks
            .iter()
            .map(|chunk| (chunk.id.as_str(), chunk))
            .collect::<HashMap<_, _>>();
        let section_by_id = corpus
            .sections
            .iter()
            .map(|section| (section.id.as_str(), section))
            .collect::<HashMap<_, _>>();
        let document_by_id = corpus
            .documents
            .iter()
            .map(|document| (document.id.as_str(), document))
            .collect::<HashMap<_, _>>();
        let mut scored = index
            .entries
            .iter()
            .filter_map(|entry| {
                let chunk = chunk_by_id.get(entry.chunk_id.as_str())?.to_owned().clone();
                let section =
                    RetrievalSection::from(*section_by_id.get(entry.section_id.as_str())?);
                let document =
                    RetrievalDocument::from(*document_by_id.get(entry.document_id.as_str())?);
                let score = self.index.score(&request.query, entry);
                Some(RetrievalHit {
                    citation: Citation {
                        document_title: document.title.clone(),
                        section_heading: section.heading.clone(),
                        chunk_id: chunk.id.clone(),
                        excerpt: excerpt(&chunk.text),
                    },
                    chunk,
                    section,
                    document,
                    score,
                })
            })
            .collect::<Vec<_>>();

        scored.sort_by(|left, right| right.score.total_cmp(&left.score));
        let hits = scored.into_iter().take(top_k).collect::<Vec<_>>();
        if hits.is_empty() {
            return Err(AppError::Index(
                "index is empty or no retrievable chunks were found".into(),
            ));
        }
        Ok(hits)
    }

    /// Summary-style questions need broader evidence than point lookups.
    ///
    /// Why this design:
    /// - Queries such as "这本书的核心主题是什么" or "what is this document about" carry weak
    ///   lexical overlap, so pure chunk ranking can drift toward one vivid local passage.
    /// - Appending the first chunk from the first few body sections gives `ask` a lightweight
    ///   document-level scaffold without turning the path into full-corpus prompting.
    /// - Current limitation: this assumes section order roughly follows reading order.
    pub fn augment_with_summary_leads(
        &self,
        hits: Vec<RetrievalHit>,
        corpus: &WorkspaceCorpus,
        max_sections: usize,
    ) -> Vec<RetrievalHit> {
        let deduped_hits = dedupe_hits_by_section(hits);
        let mut existing = deduped_hits
            .iter()
            .map(|hit| hit.chunk.id.clone())
            .collect::<std::collections::HashSet<_>>();
        let mut leads = Vec::new();

        for section in &corpus.sections {
            if leads.len() >= max_sections {
                break;
            }
            let Some(first_chunk_id) = section.chunk_ids.first() else {
                continue;
            };
            if existing.contains(first_chunk_id) {
                continue;
            }
            let Some(hit) = self.hit_for_chunk_id(first_chunk_id, corpus, 0.0) else {
                continue;
            };
            if !looks_like_body_lead(&hit.chunk.text) {
                continue;
            }
            existing.insert(hit.chunk.id.clone());
            leads.push(hit);
        }

        leads.into_iter().chain(deduped_hits).collect()
    }

    fn hit_for_chunk_id(
        &self,
        chunk_id: &str,
        corpus: &WorkspaceCorpus,
        score: f32,
    ) -> Option<RetrievalHit> {
        let chunk = corpus
            .chunks
            .iter()
            .find(|chunk| chunk.id == chunk_id)?
            .clone();
        let section = RetrievalSection::from(
            corpus
                .sections
                .iter()
                .find(|section| section.id == chunk.section_id)?,
        );
        let document = RetrievalDocument::from(
            corpus
                .documents
                .iter()
                .find(|document| document.id == chunk.document_id)?,
        );
        Some(RetrievalHit {
            citation: Citation {
                document_title: document.title.clone(),
                section_heading: section.heading.clone(),
                chunk_id: chunk.id.clone(),
                excerpt: excerpt(&chunk.text),
            },
            chunk,
            section,
            document,
            score,
        })
    }
}

fn excerpt(text: &str) -> String {
    let compact = text.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.chars().take(180).collect()
}

fn dedupe_hits_by_section(hits: Vec<RetrievalHit>) -> Vec<RetrievalHit> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for hit in hits {
        if seen.insert(hit.section.id.clone()) {
            deduped.push(hit);
        }
    }
    deduped
}

fn looks_like_body_lead(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 80 {
        return false;
    }

    let cjk_count = trimmed.chars().filter(|ch| is_cjk(*ch)).count();
    if cjk_count < 12 {
        return false;
    }

    let lower = trimmed.to_ascii_lowercase();
    !(lower.contains("isbn")
        || lower.contains("cip")
        || trimmed.contains("图书在版编目")
        || trimmed.contains("目录"))
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
    use super::*;
    use crate::{
        config::IndexConfig,
        domain::{chunk::Chunk, document::Document, section::Section},
        index::simple_index::SimpleIndex,
        storage::workspace_store::WorkspaceCorpus,
    };
    use chrono::Utc;
    use std::path::PathBuf;

    #[test]
    fn retrieve_keeps_hits_lightweight_for_large_sections() {
        let large_section_text = "A".repeat(20_000);
        let chunk_text = "ritual reform and political transition".repeat(30);
        let document = Document {
            id: "doc-1".into(),
            title: "Jian Shang".into(),
            source_path: PathBuf::from("/tmp/source.pdf"),
            stored_path: PathBuf::from("/tmp/workspace/source.pdf"),
            media_type: "application/pdf".into(),
            imported_at: Utc::now(),
            section_ids: vec!["sec-1".into()],
            chunk_ids: vec!["chunk-1".into()],
        };
        let section = Section {
            id: "sec-1".into(),
            document_id: document.id.clone(),
            heading: "Collapsed PDF Section".into(),
            level: 1,
            ordinal_path: vec![1],
            parent_id: None,
            content: large_section_text.clone(),
            chunk_ids: vec!["chunk-1".into()],
        };
        let chunk = Chunk {
            id: "chunk-1".into(),
            document_id: document.id.clone(),
            section_id: section.id.clone(),
            ordinal: 0,
            text: chunk_text.clone(),
            token_count: 12,
            keywords: vec!["ritual".into(), "transition".into()],
        };
        let corpus = WorkspaceCorpus {
            documents: vec![document],
            sections: vec![section],
            chunks: vec![chunk.clone()],
        };
        let index = SimpleIndex::new(IndexConfig {
            chunk_size: 700,
            chunk_overlap: 120,
            embedding_dimensions: 32,
            top_k: 4,
        })
        .build(&[chunk]);
        let retriever = Retriever::new(
            SimpleIndex::new(IndexConfig {
                chunk_size: 700,
                chunk_overlap: 120,
                embedding_dimensions: 32,
                top_k: 4,
            }),
            4,
        );

        let hits = retriever
            .retrieve(
                &RetrievalRequest {
                    query: "ritual transition".into(),
                    top_k: 1,
                },
                &index,
                &corpus,
            )
            .unwrap();

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chunk.text, chunk_text);
        assert_eq!(hits[0].section.heading, "Collapsed PDF Section");
        assert_eq!(hits[0].document.title, "Jian Shang");
        assert!(hits[0].citation.excerpt.len() <= 180);
        assert!(large_section_text.len() > hits[0].citation.excerpt.len());
    }

    #[test]
    fn augment_with_summary_leads_adds_opening_sections() {
        let document = Document {
            id: "doc-1".into(),
            title: "Jian Shang".into(),
            source_path: PathBuf::from("/tmp/source.pdf"),
            stored_path: PathBuf::from("/tmp/workspace/source.pdf"),
            media_type: "application/pdf".into(),
            imported_at: Utc::now(),
            section_ids: vec!["sec-1".into(), "sec-2".into()],
            chunk_ids: vec!["chunk-1".into(), "chunk-2".into()],
        };
        let section_one = Section {
            id: "sec-1".into(),
            document_id: document.id.clone(),
            heading: "第一章 新石器时代".into(),
            level: 1,
            ordinal_path: vec![1],
            parent_id: None,
            content: "first".into(),
            chunk_ids: vec!["chunk-1".into()],
        };
        let section_two = Section {
            id: "sec-2".into(),
            document_id: document.id.clone(),
            heading: "第二章 周人崛起".into(),
            level: 1,
            ordinal_path: vec![2],
            parent_id: None,
            content: "second".into(),
            chunk_ids: vec!["chunk-2".into()],
        };
        let chunk_one = Chunk {
            id: "chunk-1".into(),
            document_id: document.id.clone(),
            section_id: section_one.id.clone(),
            ordinal: 0,
            text: "殷周之变与华夏新生".repeat(20),
            token_count: 2,
            keywords: vec![],
        };
        let chunk_two = Chunk {
            id: "chunk-2".into(),
            document_id: document.id.clone(),
            section_id: section_two.id.clone(),
            ordinal: 0,
            text: "周人崛起与宗教变革".repeat(20),
            token_count: 2,
            keywords: vec![],
        };
        let corpus = WorkspaceCorpus {
            documents: vec![document],
            sections: vec![section_one, section_two],
            chunks: vec![chunk_one.clone(), chunk_two.clone()],
        };
        let retriever = Retriever::new(
            SimpleIndex::new(IndexConfig {
                chunk_size: 700,
                chunk_overlap: 120,
                embedding_dimensions: 32,
                top_k: 4,
            }),
            4,
        );
        let hits = vec![retriever.hit_for_chunk_id("chunk-2", &corpus, 1.0).unwrap()];

        let augmented = retriever.augment_with_summary_leads(hits, &corpus, 2);

        assert_eq!(augmented.len(), 2);
        assert_eq!(augmented[0].chunk.id, "chunk-1");
        assert_eq!(augmented[1].chunk.id, "chunk-2");
    }

    #[test]
    fn augment_with_summary_leads_dedupes_same_section_hits() {
        let document = Document {
            id: "doc-1".into(),
            title: "Jian Shang".into(),
            source_path: PathBuf::from("/tmp/source.pdf"),
            stored_path: PathBuf::from("/tmp/workspace/source.pdf"),
            media_type: "application/pdf".into(),
            imported_at: Utc::now(),
            section_ids: vec!["sec-1".into()],
            chunk_ids: vec!["chunk-1".into(), "chunk-2".into()],
        };
        let section = Section {
            id: "sec-1".into(),
            document_id: document.id.clone(),
            heading: "第一章 新石器时代".into(),
            level: 1,
            ordinal_path: vec![1],
            parent_id: None,
            content: "正文".into(),
            chunk_ids: vec!["chunk-1".into(), "chunk-2".into()],
        };
        let chunk_one = Chunk {
            id: "chunk-1".into(),
            document_id: document.id.clone(),
            section_id: section.id.clone(),
            ordinal: 0,
            text: "殷周之变与华夏新生".repeat(20),
            token_count: 20,
            keywords: vec![],
        };
        let chunk_two = Chunk {
            id: "chunk-2".into(),
            document_id: document.id.clone(),
            section_id: section.id.clone(),
            ordinal: 1,
            text: "人祭与王权".repeat(20),
            token_count: 20,
            keywords: vec![],
        };
        let corpus = WorkspaceCorpus {
            documents: vec![document],
            sections: vec![section],
            chunks: vec![chunk_one.clone(), chunk_two.clone()],
        };
        let retriever = Retriever::new(
            SimpleIndex::new(IndexConfig {
                chunk_size: 700,
                chunk_overlap: 120,
                embedding_dimensions: 32,
                top_k: 4,
            }),
            4,
        );
        let hits = vec![
            retriever.hit_for_chunk_id("chunk-1", &corpus, 1.0).unwrap(),
            retriever.hit_for_chunk_id("chunk-2", &corpus, 0.9).unwrap(),
        ];

        let augmented = retriever.augment_with_summary_leads(hits, &corpus, 1);

        assert_eq!(augmented.len(), 1);
        assert_eq!(augmented[0].section.id, "sec-1");
    }
}
