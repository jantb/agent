# Agent — Local AI TUI

Rust TUI agent powered by Ollama, built with ratatui + tokio + crossterm.

## Architecture

- **Structured concurrency** via tokio: main event loop uses `tokio::select!` over terminal events, agent event channel, and tick timer. Agent task is spawned separately and communicates via mpsc channels (`AgentEvent` downstream, `UserAction` upstream).
- **No shared mutable state** — App state lives in the main loop; agent task owns history and session. Communication is message-passing only.
- Tool execution uses `tokio::task::spawn_blocking` for file I/O and `tokio::process::Command` for subprocesses.

## Module layout

```
src/
  main.rs       — CLI args, terminal setup, event loop
  agent.rs      — AgentTask: turn loop, tool dispatch, session save
  ollama.rs     — Ollama HTTP streaming client, think-tag parsing
  mcp.rs        — MCP server connections and tool execution
  tools/
    mod.rs      — Tool definitions, dispatch, path sandboxing
    builtin.rs  — All built-in tool implementations
  app.rs        — UI state (messages, scroll, streaming flags)
  ui.rs         — Ratatui rendering
  types.rs      — Core types (Message, ToolCall, AgentEvent, etc.)
  session.rs    — Session persistence (.agent/session.json)
  memory.rs     — Persistent memory storage (.agent/memory/)
  input.rs      — Text input state (cursor, history, paste)
  keys.rs       — Keybinding map
  markdown.rs   — Markdown-to-ratatui renderer
  config.rs     — .mcp.json parsing
```

## Conventions

- Idiomatic compact Rust. No unnecessary abstractions.
- All file tools are sandboxed to the working directory via `resolve_safe()`.
- Logging via `tracing` to `.agent/agent.log` (non-blocking file appender). Use `RUST_LOG=trace` for verbose output.
- Tests live in `#[cfg(test)] mod tests` at the bottom of each file.
- Session data in `.agent/` (gitignored).

## Build & test

```sh
cargo build
cargo test
```

<!-- terrarium-managed:start -->
<!-- terrarium-version: 0.1.0 -->
# Terrarium Rust Tools

This project uses the `rust` terrarium preset. During `terrarium run`, use the built-in terrarium MCP tools for Rust workflow commands instead of direct shell commands when possible.

Prefer the terrarium MCP tools over direct shell commands for normal Rust build workflows:

- `cargo_check`
- `cargo_test`
- `cargo_build`
- `cargo_clippy`
- `cargo_fmt_check`
- `cargo_fmt`
- `cargo_update`
- `cargo_upgrade_incompatible`
- `make_install` (only when Makefile exists)
- `git_status`
- `git_unmerged`
- `git_log`
- `git_diff`
- `git_show`
- `report_violation`

You are running inside a macOS sandbox with a deny-default policy. If you encounter a permission denied error or an operation blocked by the sandbox, call `report_violation` with a description of what was attempted and the error.

Use the built-in git MCP tools for read-only repository inspection instead of direct shell `git` commands.
When implementing a Rust feature, keep `Cargo.toml` aligned with the work. If dependencies or crate features changed, update `Cargo.toml` and refresh `Cargo.lock` with `cargo_update`.
`cargo_test` can run the full suite, a single test, or a filtered subset.
After implementing a Rust feature, run the tests with `cargo_test`.
Never modify tests to make them pass — tests are the source of truth. Fix the implementation to satisfy the tests. Only change a test if it does not compile. If you believe a test's intention is wrong, ask the user before changing it.
When resolving merge conflicts, diff against the source branch and verify the intention of the code is preserved. Ask the user if anything is unclear.
Always fix all compiler warnings and clippy lints — treat warnings as errors.
<!-- terrarium-managed:end -->

