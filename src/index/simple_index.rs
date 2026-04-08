use crate::{
    config::IndexConfig,
    domain::chunk::Chunk,
    utils::text::{cosine_similarity, keywords_for_text, tokenize},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// SimpleIndex keeps retrieval local and inspectable.
///
/// Why this design:
/// - An MVP research harness benefits more from transparent ranking than from a complex vector
///   database, because debugging ingest and citations is easier when the index is a JSON file.
/// - We store deterministic hashed embeddings plus keywords so retrieval can combine vector-like
///   similarity with exact term overlap.
/// - An alternative would be SQLite FTS or an external vector store, but both add operational
///   surface area before the retrieval contract is stable.
/// - Current limitation: the embedding is a lightweight approximation, not a semantic model.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SimpleIndexData {
    pub entries: Vec<IndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub chunk_id: String,
    pub document_id: String,
    pub section_id: String,
    pub keywords: Vec<String>,
    pub embedding: Vec<f32>,
}

pub struct SimpleIndex {
    config: IndexConfig,
}

impl SimpleIndex {
    pub fn new(config: IndexConfig) -> Self {
        Self { config }
    }

    pub fn build(&self, chunks: &[Chunk]) -> SimpleIndexData {
        SimpleIndexData {
            entries: chunks
                .iter()
                .map(|chunk| IndexEntry {
                    chunk_id: chunk.id.clone(),
                    document_id: chunk.document_id.clone(),
                    section_id: chunk.section_id.clone(),
                    keywords: keywords_for_text(&chunk.text, 10),
                    embedding: self.embed(&chunk.text),
                })
                .collect(),
        }
    }

    pub fn merge(&self, existing: SimpleIndexData, chunks: &[Chunk]) -> SimpleIndexData {
        let mut map: HashMap<String, IndexEntry> = existing
            .entries
            .into_iter()
            .map(|entry| (entry.chunk_id.clone(), entry))
            .collect();
        for entry in self.build(chunks).entries {
            map.insert(entry.chunk_id.clone(), entry);
        }
        SimpleIndexData {
            entries: map.into_values().collect(),
        }
    }

    pub fn score(&self, query: &str, entry: &IndexEntry) -> f32 {
        let query_embedding = self.embed(query);
        let vector_score = cosine_similarity(&query_embedding, &entry.embedding);
        let query_terms = tokenize(query);
        let overlap = query_terms
            .iter()
            .filter(|term| entry.keywords.iter().any(|keyword| keyword == *term))
            .count() as f32;
        vector_score + overlap * 0.12
    }

    fn embed(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0.0; self.config.embedding_dimensions];
        for token in tokenize(text) {
            let hash = token.bytes().fold(0usize, |acc, byte| {
                acc.wrapping_mul(31).wrapping_add(byte as usize)
            });
            let slot = hash % self.config.embedding_dimensions;
            vector[slot] += 1.0;
        }
        normalize(vector)
    }
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}
