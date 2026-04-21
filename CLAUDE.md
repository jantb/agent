# Agent — Local AI TUI

Rust TUI agent powered by Ollama, built with ratatui + tokio + crossterm.

## Architecture

- **Structured concurrency** via tokio: main event loop uses `tokio::select!` over terminal events, agent event channel, and tick timer. Agent task is spawned separately and communicates via mpsc channels (`AgentEvent` downstream, `UserAction` upstream).
- **No shared mutable state** — App state lives in the main loop; agent task owns history and session. Communication is message-passing only.
- Tool execution uses `tokio::task::spawn_blocking` for file I/O and `tokio::process::Command` for subprocesses.

## Module layout

```
src/
  main.rs          — thin entry: Cli + fn main → dispatches to headless or tui
  bootstrap.rs     — setup(): logging, ollama, MCP, session load, agent spawn
  headless.rs      — run_script(): script/headless mode runner
  lib.rs           — module declarations
  agent/           — turn loop + tool dispatch
    mod.rs         — AgentTask struct, root run loop
    turn.rs        — execute_turn, handle_tool_calls, handle_update_plan
    subtask.rs     — run_subtask, run_single_task, SubtaskExitGuard
    loop_detect.rs — text_fingerprint, check_repeated_text, truncate_subtask_result
  app/             — UI state (no rendering)
    mod.rs         — App struct + constructor
    messages.rs    — chat message ops, queue, pending images
    streaming.rs   — streaming state + token stats
    scroll.rs      — scroll viewport
    tree.rs        — subtask tree state
    plan.rs        — plan state
    pickers.rs     — ModelPickerState, InterviewPickerState
  ollama/          — Ollama HTTP client
    mod.rs         — OllamaClient, stream_turn, NUM_CTX
    stream.rs      — LineParser, ThinkTagFilter
    parse.rs       — parse_context_window
  prompts.rs       — system prompt builders + mode appendices
  session/         — persisted conversation state
    mod.rs         — Session struct, SessionMessage enum
    persist.rs     — load, save, save_subtask
    history.rs     — to_ollama_history, to_compressed_history
    convert.rs     — SessionMessage → ChatMessage conversion
    gitignore.rs   — ensure_gitignore
  tools/           — tool definitions + dispatch
    mod.rs         — re-exports, IGNORE_DIRS, PLAN_WRITE_TOOLS, resolve_safe
    definitions.rs — JSON schemas for all tools
    dispatch.rs    — execute_built_in_with_mode
    selection.rs   — tools_for_depth, is_flat_model
    builtin/       — builtin tool impls (file_io, memory_tools, search, text_ops)
  tui/             — TUI event loop + command executor
    mod.rs         — run_loop, TerminalGuard
    events.rs      — AgentEvent → App mutation
    commands.rs    — slash commands, key-driven actions
  ui/              — ratatui rendering (pure, takes &App)
    mod.rs         — draw coordinator
    chat.rs, input.rs, status.rs, tree.rs, title.rs, picker.rs, util.rs
  memory.rs, mcp.rs, types.rs, keys.rs (keymap), input.rs, markdown.rs,
  autocomplete.rs, config.rs, highlight.rs, script.rs
```

## Conventions

- Idiomatic compact Rust. No unnecessary abstractions.
- All file tools are sandboxed to the working directory via `resolve_safe()` in `src/tools/mod.rs`.
- Logging via `tracing` to `.agent/agent.log` (non-blocking file appender; `WorkerGuard` held in `Setup`). Use `RUST_LOG=trace` for verbose output.
- Tests live in `#[cfg(test)] mod tests` at the bottom of each file.
- Session data in `.agent/` (gitignored).

## Security

- `resolve_safe()` (in `src/tools/mod.rs`) sandboxes all file tool paths to the working directory using lexical normalization + canonicalized-root prefix check.
- **Known limitation**: symlinks inside the working directory that point outside are NOT detected. The lexical path stays under the prefix so `starts_with` passes; the OS resolves the symlink at open-time. For strict isolation, run the agent in a chroot or container.

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

