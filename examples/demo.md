# Research Harness Demo

## Purpose

Research Harness is a local CLI workflow for document-centric research. It ingests source files,
rebuilds section structure, chunks content for retrieval, and answers questions with citations.

## Why Structure Matters

The system keeps both sections and chunks. Sections preserve author-facing hierarchy so answers can
cite meaningful locations. Chunks are smaller retrieval windows so the index can rank relevant
evidence without dragging an entire chapter into each query.

For the `ask` command, the model sees retrieved chunk text, not the full section body. Section
metadata is carried alongside the chunk so the answer can cite a human-readable heading without
re-inflating the prompt.

## MVP Constraints

The MVP does not depend on an external vector database. Instead, it uses a deterministic local
index that combines hashed embeddings with keyword overlap. This keeps the workflow transparent and
easy to debug while the ingestion and citation pipeline is being validated.

PDF support now prefers actual text extraction instead of treating the file as raw bytes. That does
not replace OCR, but it is enough to recover readable text from many text-based books that were
previously ingested as binary noise.

When a PDF is image-only, the harness can also fall back to local OCR if `pdftoppm` and
`tesseract` are available. This OCR path is intentionally bounded so imports stay reviewable.

## Expected Outputs

The `ask` command should separate original content, system summary, inference, and citations.
Reports should create structured summaries, comparisons, and outlines across all imported
documents in the active workspace.

This is an intentional split:

- `ask` is chunk-retrieval-first
- `report` is section-aggregation-first
