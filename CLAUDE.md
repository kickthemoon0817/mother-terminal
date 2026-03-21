# Project Rules — MTT (AI-Native Terminal Multiplexer)

## Language & Edition
- **Rust 2024 edition** — always use the latest Rust edition features
- **Safe Rust only** — no `unsafe` blocks unless absolutely required and documented with a safety comment
- Prefer safe abstractions over raw pointer manipulation

## Build & Test
- `cargo build` must produce zero errors
- `cargo clippy` must produce zero warnings
- `cargo test` must pass all tests
- Use `#[deny(unsafe_code)]` at crate level

## Architecture
- AI-native terminal multiplexer using PTY + ratatui
- Each AI CLI session runs in its own PTY (full colors, arrow keys, native terminal)
- Status bar + command bar are rendered by ratatui
- No terminal nesting — mtt IS the terminal

## Dependencies
- `crossterm` — terminal control
- `ratatui` — TUI rendering
- `portable-pty` — PTY management
- `vt100` — ANSI escape sequence parsing / virtual screen
- `tokio` — async runtime
- `clap` — CLI argument parsing
- `serde` + `toml` — config
- `anyhow` — error handling

## Versioning
- Never exceed v0.1.0 — stay in 0.0.x range
- GitHub: kickthemoon0817/mother-terminal

## Code Style
- No `unwrap()` in production code — use `?` or handle errors explicitly
- `unwrap()` is acceptable only in tests
- Prefer `anyhow::Result` for functions that can fail
- Use descriptive error messages with `anyhow::bail!` or `anyhow::Context`
