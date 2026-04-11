# Agent

A high-performance, terminal-based AI agent application written in Rust. This project provides an interactive environment for managing AI-driven tasks and sessions directly within your terminal, featuring a sophisticated TUI and extensible tool capabilities.

## Features

- **Interactive TUI**: A rich Terminal User Interface built with `ratatui` and `crossterm`, providing an immersive command-line experience.
- **Modular Architecture**: Decoupled design for agent logic, session management, and memory.
- **Extensible Integrations**:
    - **LLM Support**: Native integration with backends like Ollama.
    - **Model Context Protocol (MCP)**: Support for MCP to extend agent capabilities via pluggable tools.
- **Advanced Capabilities**:
    - Robust session and memory management.
    - Markdown rendering and syntax highlighting.
    - Clipboard integration and image handling.
    - Asychronous core powered by `tokio`.

## Technology Stack

- **Language**: Rust
- **Async Runtime**: `tokio`
- **UI Framework**: `ratatui` & `crossterm`
- **Communication**: `reqwest` (HTTP), `mcp` (Model Context Protocol)
- **Data Handling**: `serde` (Serialization), `regex` (Pattern matching)
- **CLI Parsing**: `clap`

## Project Structure

- `src/agent.rs`: Core agent logic and orchestration.
- `src/ui.rs`: Terminal User Interface implementation.
- `src/session.rs`: Session management and state persistence.
- `src/memory.rs`: Agent memory and context management.
- `src/mcp.rs`: Integration with the Model Context Protocol.
- `src/ollama.rs`: Integration with Ollama LLM backend.
- `tools/`: Directory for plugin and tool definitions.

## Getting Started

[Insert instructions on how to clone, build (e.g., `cargo build`), and run the application here.]
