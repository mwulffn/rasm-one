# Rust Project: rasm-one

## Build & Test Commands
- Build: `cargo build`
- Test: `cargo test`
- Test with output: `cargo test -- --nocapture`
- Lint: `cargo clippy -- -D warnings`
- Format: `cargo fmt`
- Check (fast, no codegen): `cargo check`
- Audit dependencies: `cargo audit`
- Documentation: `cargo doc --open`

## Rust Edition and Toolchain
- Edition: 2021 (always specify in Cargo.toml)
- Toolchain: a recent stable, pinned exactly in rust-toolchain.toml and matched by `rust-version` in Cargo.toml. Update both together, deliberately
- Do NOT use nightly features

## Error Handling
- Libraries: use `thiserror` — derive `Error` for all custom error types
- Applications/binaries: use `anyhow` — propagate with `?`, add context with `.context()`
- Never use `.unwrap()` or `.expect()` in library code
- `.expect()` is acceptable in binary main() for setup failures (file not found, etc.)
- Never use `.unwrap()` in tests — use `?` with `#[tokio::test]` or return `Result<(), Box<dyn Error>>`
- Propagate errors with `?` unless there is a documented reason to handle locally

## Clippy Policy
- Treat all warnings as errors: `cargo clippy -- -D warnings`
- Run clippy before committing, not just before merging
- Do not use `#[allow(clippy::...)]` without a comment explaining why

## Code Style
- All public items (structs, enums, functions, traits, modules) must have doc comments (`///`)
- Private items: doc comments are encouraged but not required
- Prefer explicit type annotations on public function signatures
- Do not use wildcard imports (`use foo::*`) except in test modules

## Dependency Policy
- Prefer minimal dependencies — justify each new crate in the PR description
- Pin exact versions for production binaries (`=1.2.3` in Cargo.toml)
- Use `^` (caret) for library crates (allows compatible updates)
- Run `cargo audit` before every release
- Do not add a crate to solve a problem that can be solved with std in under 20 lines

## Testing
- Unit tests: `#[cfg(test)]` module at the bottom of each file
- Integration tests: `tests/` directory
- Test naming: `test_<function_name>_<scenario>` pattern
- Use `cargo test -- --nocapture` when debugging output
- Mock external dependencies using trait objects, not concrete types
- Aim for 80%+ coverage on library crates; focus on edge cases not happy paths

## Memory Safety and Unsafe
- No `unsafe` blocks without a `// SAFETY:` comment explaining the invariants upheld
- If `unsafe` is required, isolate it in a dedicated module (e.g., `src/ffi/` or `src/sys/`)
- Prefer `Arc<Mutex<T>>` over raw pointers for shared state across threads
- Do not use `Rc<RefCell<T>>` in async code (not `Send`)
- If you need `unsafe`, ask before writing it — we may have a safe alternative

## Module Structure
- `src/lib.rs` or `src/main.rs` — crate root, minimal logic
- `src/error.rs` — all custom error types
- `src/types.rs` — shared types and newtypes
- Keep modules focused: one concern per file, not one per type
