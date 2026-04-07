## Architecture

### Module responsibilities

| Module | Role |
|---|---|
| `main.rs` | Entry point, terminal setup, event loop — thin dispatcher only |
| `keys.rs` | Key-to-command mapping — pure function, no I/O, no async |
| `markdown.rs` | Markdown-to-styled-lines renderer — pure function, no I/O, no async |
| `types.rs` | Shared domain types: `Role`, `Message`, `ToolCall`, `ToolResult`, `AgentEvent` |
| `config.rs` | Load and parse `.mcp.json` |
| `ollama.rs` | Ollama HTTP client, streaming, think-tag parsing |
| `tools.rs` | Built-in tools: `read_file`, `write_file`, `list_dir`, `edit_file`, `search_files` |
| `mcp.rs` | MCP JSON-RPC client, tool discovery, execution |
| `agent.rs` | Agent loop: turn management, tool dispatch |
| `session.rs` | Session persistence (`.agent/session.json`) |
| `input.rs` | Input editing, cursor, history cycling — pure state, no I/O |
| `app.rs` | UI state: messages, streaming, scroll, tool display — pure state, no I/O |
| `ui.rs` | Ratatui rendering — reads `App`, never mutates it |

### Layering rules

- `types.rs` has no imports from other project modules.
- `app.rs` and `input.rs` are pure state — no I/O, no async, no network calls.
- `ui.rs` takes `&App` — read-only; never mutates state.
- `main.rs` is thin wiring — no business logic, only glues modules together.
- `agent.rs` owns the conversation loop and all tool dispatch.
- All filesystem ops in `tools.rs` must go through `resolve_safe()` for sandboxing to the working directory.

### Key conventions

- All tool file access sandboxed to working directory via `resolve_safe()`.
- Use `tokio::fs` or `spawn_blocking` for filesystem I/O in async context.
- Session saved after every message — crash-safe.
- MCP uses JSON-RPC with an atomic counter for request IDs.
- Think-tag parsing (`<think>`/`</think>`) lives in `ollama.rs` only — nowhere else.
- Streaming JSON from Ollama must be line-buffered across TCP chunks.

### Adding a new built-in tool

1. Add definition in `tools.rs::built_in_tool_definitions()`.
2. Add handler branch in `execute_built_in()`.
3. Use `resolve_safe()` for every path argument.

### Structured concurrency pattern

This is the standard pattern for all event-driven code in this project:

- **Main event loop is a thin dispatcher.** The `loop` body should be ~10-15 lines: tick, draw, `tokio::select!` over event sources, break check. No business logic inline.
- **Key handling is a pure function.** `keys::map_key(KeyEvent, streaming) -> UiCommand` maps terminal events to a `UiCommand` enum. No side effects, no async, trivially testable.
- **Commands are applied via `apply_command`.** A single `match` on `UiCommand` where each arm is 1-3 lines calling `App` methods or sending `UserAction` over the channel.
- **Agent events are handled via `handle_agent_event`.** A pure function matching `AgentEvent` variants to `App` method calls. No async needed.
- **Agent task uses phase methods.** `run()` is a skeleton loop; actual work lives in `execute_turn()`, `handle_tool_calls()`, `handle_text_turn()`. Each phase method is independently readable.
- **Never block the UI thread.** Any I/O in the event loop (clipboard, filesystem) goes through `tokio::task::spawn_blocking`. The only `await` points in the main loop are channel recv and terminal event stream.
- **Communication is channels only.** `UserAction` (UI → agent), `AgentEvent` (agent → UI). No shared mutable state crosses task boundaries.

### Adding a new keybinding

1. Add variant to `UiCommand` in `keys.rs`.
2. Add match arm in `map_key()` with a test.
3. Add handler in `apply_command()` in `main.rs`.

### Adding a new slash command

1. Add detection in the `Submit` handler of `apply_command()`.
2. If it needs agent cooperation, add a `UserAction` variant in `agent.rs` and handle it in `run()`.

### Adding a new module

Keep it focused on one concern. Update the module table in this file.

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

