# Agent — Local AI TUI

A powerful, private, and local-first AI agent for your terminal. Built with Rust, powered by Ollama, and designed for seamless tool execution and long-term memory.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
![Rust](https://img.shields.io/badge/rust-%23000000.svg?style=flat&logo=rust&logoColor=white)

## 🤖 Overview

**Agent** is a Terminal User Interface (TUI) that brings the power of LLMs directly into your development workflow. Unlike web-based chat interfaces, Agent lives in your terminal, understands your local filesystem, and can execute tools to help you automate tasks, manage files, and interact with external services via the **Model Context Protocol (MCP)**.

Everything runs locally. Your data, your files, and your models stay on your machine.

## ✨ Key Features

- 🔒 **Local-First & Private:** Powered by [Ollama](https://ollama.com/), ensuring your prompts and data never leave your hardware.
- 🛠️ **Extensible Toolset:** Out-of-the-box support for filesystem operations, subprocess execution, and more.
- 🔌 **MCP Integration:** Connect to any Model Context Protocol (MCP) server to extend Agent's capabilities with external data and tools.
- 🧠 **Persistent Memory & Sessions:** Agent maintains long-term memory and session history, allowing for continuous, context-aware workflows.
- 🎨 **Rich TUI Experience:** A beautiful, high-performance interface built with `ratatui` and `tokio`, featuring full Markdown rendering.
- ⚡ **Slash Command Autocomplete:** Type `/` to get an interactive autocomplete popup for built-in commands.
- 🛡️ **Secure by Design:** All file-based tools are strictly sandboxed to the current working directory using safe path resolution.

## 🚀 Getting Started

### Prerequisites

- [Rust toolchain](https://rustup.rs/) (latest stable)
- [Ollama](https://ollama.com/) (running locally with a model pulled, e.g. `ollama pull gemma4:26b`)

### Installation

Clone the repository and install via Cargo:

```bash
git clone https://github.com/youruser/agent.git
cd agent
cargo install --path .
```

### Usage

Simply run the agent in your terminal:

```bash
agent                              # uses default model (gemma4:26b)
agent --model llama3               # specify a model
agent --ollama-url http://host:11434  # custom Ollama endpoint
```

Once running, you can chat with the agent, ask it to read files, run shell commands, or explore your directory.

### Keybindings

| Key | Action |
|-----|--------|
| `Enter` | Send message |
| `Shift+Enter` | Insert newline |
| `Tab` | Autocomplete `/commands` |
| `Esc` | Cancel streaming / dismiss autocomplete |
| `Ctrl+C` | Quit |
| `Ctrl+U` | Clear input |
| `Ctrl+W` | Delete word |
| `Ctrl+A` / `Ctrl+E` | Start / end of line |
| `Ctrl+V` | Paste image from clipboard |
| `Up` / `Down` | Input history |
| `Shift+Up` / `Shift+Down` | Scroll chat |
| `PageUp` / `PageDown` | Scroll page |

### Slash Commands

Type `/` as the first character to open the autocomplete popup, then filter by typing more characters.

| Command | Description |
|---------|-------------|
| `/clear` | Clear conversation and reset session |
| `/new` | Alias for `/clear` |
| `/help` | Show help text |

## ⚙️ Configuration

### MCP Servers

You can extend Agent's capabilities by defining MCP servers in a `.mcp.json` file in your project root or home directory.

```json
{
  "mcpServers": {
    "my-server": {
      "command": "node",
      "args": ["/path/to/server/index.js"]
    }
  }
}
```

## 🏗️ Architecture

Agent is built for high-performance, asynchronous execution using a modern Rust stack:

- **Concurrency:** Uses `tokio` for structured concurrency. The main event loop uses `tokio::select!` to handle terminal events, agent updates, and timers without blocking.
- **Communication:** Implements a message-passing architecture. The UI and the Agent logic communicate via `mpsc` channels, avoiding shared mutable state.
- **Rendering:** Built with `ratatui` for a responsive and flicker-free terminal UI.
- **Observability:** Integrated with `tracing` for detailed, non-blocking logging to `.agent/agent.log`.

## 🛠️ Development

If you want to contribute or explore the internals:

1. **Build the project:** `cargo build`
2. **Run tests:** `cargo test`
3. **Run with verbose logging:** `RUST_LOG=trace agent`

## 📜 License

This project is licensed under the MIT License.
