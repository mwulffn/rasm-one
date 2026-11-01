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
- Never use `.unwrap()` in tests — return `Result<(), Box<dyn Error>>` and use `?`
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
- vasm is the reference: rasm-one's output must match it byte for byte, except where vasm has a bug. Never copy a bug: do the correct thing, add a test, and add it to `docs/vasm-bugs.md`. Differential tests live in `crates/rasm-one/tests/vasm_*.rs` and `sweep_cpus.rs`, using the `rasm-oracle` crate
- Get vasm with `export RASM_VASM=$(scripts/fetch-vasm.sh)`. Never commit vasm or its sources (licence)
- Without `RASM_VASM`, differential tests skip locally and fail in CI. A green local run without it proves little
- The oracle runs vasm with `-no-opt`. Optimisations are added one rule at a time, each matched to its vasm flag (#20)
- New instructions or addressing modes go in the sweep, not as hand-picked cases
- Feed sources through `MemoryProvider`, not files on disk
- Performance budget: `cargo test --release -p rasm-one --test perf_budget`. It skips in debug builds
- Unit tests: `#[cfg(test)]` module at the bottom of each file; integration tests in `tests/`
- Test naming: `test_<function_name>_<scenario>`

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
