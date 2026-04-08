# Architecture

## System Flow

The MVP is a single-process Rust CLI. The main flow is:

1. Load config and resolve the active workspace
2. Ingest a source document or a research query
3. Parse normalized text into sections
4. Split sections into chunks
5. Build or load the local index
6. Retrieve relevant chunks for a query
7. Synthesize an answer or workspace report
8. Save outputs and append workspace logs

Textually, the pipeline is:

`ingest -> parse -> index -> retrieve -> synthesize -> output`

## Data Flow

### Ingest

- `src/ingest/` reads `pdf`, `md`, or `txt`
- Output is normalized text plus original bytes for workspace persistence
- PDF ingest now prefers a real text extractor and only falls back to byte scanning when readable
  text cannot be recovered directly
- When both paths fail, ingest distinguishes likely image-only PDFs from text PDFs that simply have
  poor extraction quality and returns a more actionable error
- For image-only PDFs, ingest can optionally rasterize a bounded number of pages and OCR them with
  local `pdftoppm` and `tesseract`
- The OCR path rasterizes in grayscale at higher resolution and evaluates two Tesseract page
  segmentation modes, keeping the more readable candidate for each page
- After OCR, ingest trims obvious front matter before the first numbered chapter so ISBN and CIP
  metadata do not dominate retrieval
- OCR cleanup also removes repeated layout noise such as running headers, figure captions, and
  short garbage lines before section parsing
- OCR cleanup also strips short Latin garbage tokens from otherwise Chinese prose lines
- When OCR splits one logical chapter into consecutive sections with the same heading, ingest
  merges those adjacent sections back together before chunking

### Parse

- `src/parser/structure.rs` rebuilds headings into `Section`
- Each `Section` preserves hierarchy metadata for later citation
- The heading heuristic covers markdown headings, uppercase ASCII headings, and common Chinese
  chapter markers such as `第一章` and `第七节`

### Chunk

- `src/ingest/mod.rs` creates `Chunk`
- Chunks are small retrieval windows linked back to a parent section

### Index

- `src/index/simple_index.rs` stores deterministic hashed embeddings and keywords in JSON
- This avoids introducing a complex vector database before retrieval behavior is stable
- Tokenization supports ASCII terms and CJK bigrams so Chinese OCR text can participate in the same
  local ranking path

### Retrieve

- `src/retrieval/retriever.rs` ranks chunks for a query
- Each retrieval hit is expanded back to lightweight document and section metadata
- The result includes a `Citation` with document, section, and excerpt
- The `ask` path keeps only chunk text as LLM evidence, so retrieval does not inflate prompts by
  copying full section bodies back into every hit
- For summary-style questions, retrieval also appends lead chunks from the first few body sections
  to keep document-level context without switching to full-corpus prompting

### Synthesize

- `src/synthesis/synthesizer.rs` formats evidence into:
  - original content
  - system summary
  - inference
  - citations
- `ask` uses retrieved chunk packets as evidence
- `report` uses section packets because it summarizes the full workspace
- Citation fidelity is prioritized over answer polish because this is a research system

## Module Responsibilities

- `app.rs`: command orchestration
- `cli.rs`: Clap command definitions
- `domain/`: stable data contracts
- `ingest/`: file-type specific text extraction
- `parser/`: structure recovery
- `index/`: local retrieval index
- `retrieval/`: ranking and evidence assembly
- `synthesis/`: answer and report generation
- `llm/`: provider abstraction and implementations
- `storage/`: workspace persistence and logs
- `utils/`: text helpers

## Design Tradeoffs

### Why not a complex DB?

The MVP stores JSON files under each workspace so every artifact is easy to inspect. This makes ingest, chunking, and citations debuggable. A database can be added later once the schema and ranking behavior settle.

### Why a mock LLM first?

The main engineering risk is the document pipeline, not model integration. A mock provider keeps `cargo build`, `cargo test`, and demo runs deterministic even without API keys.

### Why `Section + Chunk`?

Sections preserve the document’s meaning and are appropriate citation anchors. Chunks are retrieval units that keep ranking specific and prompt context bounded. The explicit link between them is what allows the system to be both searchable and citeable.

The implementation detail that matters most is that `ask` retrieves chunks first and only then
attaches section metadata for provenance. It does not expand the full section body back into each
retrieval hit. That keeps large, poorly structured PDFs from turning a query into a full-document
prompt by accident.

## Extension Path

- Introduce a stronger PDF parser or OCR stage
- Add page-aware citations
- Swap the simple index for SQLite FTS or a vector backend
- Add richer report types and provider prompts
- Support document refresh or deletion within a workspace
