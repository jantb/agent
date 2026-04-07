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
