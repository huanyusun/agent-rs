# research-harness

`research-harness` is a Rust-first CLI MVP for local document research. It targets the core workflow that tools like NotebookLM provide, but keeps the system local, inspectable, and runnable without a front-end project.

## Purpose

The harness is designed for research rather than chat. It imports source documents, rebuilds structure, indexes retrievable chunks, answers questions with citations, and generates workspace-level outputs such as summaries, comparisons, and outlines.

## Relationship To NotebookLM

This project replaces the central NotebookLM loop with a local CLI pipeline:

`ingest -> parse -> chunk -> index -> retrieve -> synthesize -> cite`

The goal is not UI parity. The goal is a trustworthy document research workflow that can later support richer providers or storage backends.

## Features

- Workspace-based document collections
- Import support for `pdf`, `md`, and `txt`
- Section-aware parsing with chunk-level retrieval
- PDF ingest uses real text extraction before falling back to byte scanning
- Image-only PDFs can use a local OCR fallback when `pdftoppm` and `tesseract` are installed
- OCR now rasterizes scanned PDFs in grayscale at higher resolution and picks the better result
  from two bounded Tesseract page-segmentation modes
- Retrieval tokenization now supports both ASCII terms and CJK bigrams for Chinese queries and OCR text
- PDF ingest trims obvious OCR front matter such as ISBN and CIP blocks when numbered chapter headings are present
- PDF OCR cleanup also drops common page-header, figure-caption, and short garbage lines before section parsing
- OCR cleanup also strips short Latin garbage tokens from otherwise Chinese body lines
- Adjacent PDF sections with the same recovered heading are merged so OCR page breaks do not fragment citations
- Local JSON index with deterministic hashed embeddings plus keywords
- Question-driven retrieval with citations
- Structured reports: `summary`, `compare`, `outline`
- Mock LLM provider by default, optional OpenAI provider
- Observable outputs through workspace logs and saved markdown reports

## Ask Flow

The `ask` command is intentionally retrieval-first:

1. Ingest stores both `Section` and `Chunk`
2. The local index ranks matching chunks for the query
3. Retrieval maps each winning chunk back to document and section metadata
4. Summary-style questions also pull in the first chunk from the first few body sections
5. The LLM receives only those bounded chunk packets plus document and section labels
6. The final answer prints citations built from the same retrieval hits

This means `ask` does not concatenate every imported document into one prompt. It also does not
re-expand a full section body after retrieval just because the section is available on disk.
Section metadata is used as a citation anchor. Chunk text is the evidence that goes into answer
generation.

This distinction matters for large PDFs. A weak PDF heading parser can collapse a whole book into
one section, but `ask` still works from chunk retrieval rather than shipping the whole section text
to the model.

For Chinese non-fiction and history books, the parser now also recognizes headings such as
`第一章`, `第七节`, and `第3卷` as section boundaries when the PDF text extractor preserves them.
For scanned books, ingest also removes common layout noise such as repeated running headers,
figure-caption lines, and short ASCII garbage before those heading heuristics run.
The OCR path also prefers the more readable result across two conservative Tesseract segmentation
modes instead of assuming one fixed page layout.

## Project Layout

```text
.
├── Cargo.toml
├── README.md
├── ARCHITECTURE.md
├── config/default.toml
├── src/
│   ├── app.rs
│   ├── cli.rs
│   ├── config.rs
│   ├── domain/
│   ├── index/
│   ├── ingest/
│   ├── llm/
│   ├── parser/
│   ├── retrieval/
│   ├── storage/
│   ├── synthesis/
│   └── utils/
├── examples/demo.md
└── workspace/
```

## CLI Usage

```bash
cargo run -- workspace create demo
cargo run -- add ./examples/demo.md
cargo run -- ask "这个文档在讲什么？"
cargo run -- report summary
cargo run -- report compare
cargo run -- report outline
```

Use `cargo run -- workspace show` to inspect the active workspace. `workspace create` also marks that workspace as active, so later commands can omit the workspace name.

## Example Flow

```bash
cargo build
cargo run -- workspace create demo
cargo run -- add ./examples/demo.md
cargo run -- ask "What is the main purpose of the document?"
```

This creates `workspace/demo/` with:

- `documents/`: copied source files and document metadata
- `chunks/`: parsed sections and retrieval chunks
- `index/index.json`: local searchable index
- `outputs/`: saved answers and reports
- `logs/activity.jsonl`: observable command log

## Configuration

Default settings live in `config/default.toml`.

- `app.workspace_root`: where workspaces are created
- `app.active_workspace_file`: pointer to the active workspace
- `llm.provider`: `mock` or `openai`
- `index.chunk_size` / `index.chunk_overlap`: chunking controls

Environment variables:

- `RESEARCH_HARNESS_LOG`
- `OPENAI_API_KEY`

## How To Extend

- Replace the simple PDF extractor with a stronger parser or OCR pipeline
- Swap the local index for SQLite FTS or a vector database once retrieval quality warrants it
- Add richer synthesis prompts or new `LLMProvider` implementations without changing retrieval
- Improve section detection for PDFs and plain text while keeping the `Section -> Chunk` contract

## Current Limits

- `ask` is chunk-retrieval-first, but citations are only as good as the recovered section headings
- `report` is workspace-wide and still feeds section bodies to the LLM because it is not a
  retrieval query path
- PDF structure recovery is heuristic, so some PDFs may degrade into one giant section with many
  chunks
- Text-based PDFs work much better now, but scanned PDFs still need OCR
- OCR cleanup is line-based and heuristic, so unusual books may still keep some page furniture or
  lose a small amount of non-body text
- If a Chinese scanned PDF fails ingest, install an OCR toolchain with a Chinese language pack such
  as Tesseract `chi_sim`; image-only PDFs do not contain enough embedded text for retrieval
- OCR fallback is intentionally capped by `RESEARCH_HARNESS_PDF_OCR_MAX_PAGES` with a default of
  `24` pages so very large books do not turn one ingest into an unbounded local batch job
