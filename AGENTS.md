# Repository Guidelines

## Repository Rules
- Keep this repository Rust-first. Do not introduce JavaScript or TypeScript.
- Prefer simple, explicit, maintainable code over abstraction-heavy designs.
- Build a runnable MVP first, then refine once the core path works.
- Give every important module a single, clear responsibility.
- Keep shell execution constrained and safe by default.
- Keep file operations read-only unless the repository is explicitly extended for writes later.
- Log all major flows so runs can be inspected and replayed.
- Do not leave placeholder-only implementations in core execution paths.
- Before closing work, run build, test, and demo commands and fix failures.

## Project Structure & Module Organization
This is a single Rust crate. Use `src/main.rs` for CLI entry, `src/app.rs` for orchestration, and `src/lib.rs` for shared exports. Keep domain models in `src/domain/`, context construction in `src/context/`, planning in `src/planner/`, execution in `src/executor/`, tool integrations in `src/tools/`, persistence in `src/storage/`, and provider adapters in `src/llm/`.

Configuration lives in `config/default.toml`. Demo inputs live in `demo_task.txt` and `examples/`. Runtime artifacts such as `sessions/` and `logs/` should stay reviewable and should not be expanded casually.

## Build, Test, and Development Commands
- `cargo build`: compile the project.
- `cargo test`: run unit tests.
- `cargo run -- run "demo task"`: execute the default demo flow.
- `cargo run -- run --task-file demo_task.txt`: execute a task from file.
- `cargo fmt`: apply standard Rust formatting.
- `cargo clippy --all-targets --all-features`: catch maintainability and safety issues.

## Coding Style
- Prefer small modules with explicit names.
- Use traits when they improve current extensibility, not speculative future design.
- Add comments only where they clarify intent, safety boundaries, or architecture decisions.
- Avoid clever code that reduces maintainability.
- Follow Rust defaults: 4-space indentation, `snake_case` for functions/modules, `CamelCase` for types.

## Testing & Delivery Standard
Place tests near the code in inline `#[cfg(test)]` modules. Use descriptive names such as `run_demo_task_persists_session`, and use `#[tokio::test]` for async flows. The project is not complete unless it compiles, the demo command runs, and the README explains architecture, commands, and extension points.

## Commit & Pull Request Guidelines
Use short, imperative commit subjects, consistent with the existing history, for example `Add clipboard CLI design spec`. Keep PRs focused, summarize behavior and config changes, and list the commands you ran to validate the change.
