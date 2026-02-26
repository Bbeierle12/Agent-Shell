# Agent-Shell Python Agent

An AI agent powered by **Microsoft Agent Framework** and **Ollama** that exposes an OpenAI-compatible HTTP API. It auto-starts the Rust `agent-shell` binary as a sidecar for sandboxed code execution.

## Features

- **8 agentic tools**: file read/write, directory listing, shell commands, Python execution, regex search, web fetch, and delegation to the Rust agent-shell
- **OpenAI-compatible API** (`/v1/chat/completions`) with streaming SSE support
- **Sidecar architecture**: automatically launches the Rust binary for Docker-sandboxed execution
- **Session management**: per-session conversation threads with LRU eviction
- **Interactive CLI mode** for quick testing

## Prerequisites

- **Python 3.10+**
- **Ollama** running locally with a function-calling model (e.g. `qwen2.5`, `mistral`, `llama3.2`)
- **Rust agent-shell binary** built from the parent directory (`cargo build --release`)

## Setup

```bash
cd agent-py

# Create virtual environment and install dependencies
python3 -m venv .venv
source .venv/bin/activate
pip install -r requirements.txt

# Copy and configure environment
cp .env.example .env   # or edit .env directly
# Required: set OLLAMA_HOST and OLLAMA_MODEL_ID
```

### `.env` configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `OLLAMA_HOST` | `http://localhost:11434` | Ollama server URL |
| `OLLAMA_MODEL_ID` | `qwen2.5` | Model to use for inference |
| `AGENT_SHELL_BINARY` | `../target/release/agent-shell` | Path to Rust binary (auto-detected) |
| `AGENT_SHELL_PORT` | `8080` | Port for the Rust sidecar |
| `AGENT_SHELL_HOST` | `127.0.0.1` | Host for the Rust sidecar |
| `AGENT_SERVER_PORT` | `8087` | Port for the Python HTTP server |

## Usage

### HTTP server (default)

```bash
source .venv/bin/activate
python agent.py
# Server starts on http://127.0.0.1:8087
# Rust sidecar starts on http://127.0.0.1:8080
```

### Interactive CLI

```bash
source .venv/bin/activate
python agent.py --cli
```

### VS Code (F5)

The `.vscode/launch.json` includes configurations for both server and CLI modes. Press F5 to launch with the AI Toolkit Agent Inspector attached.

## API

### POST `/v1/chat/completions`

OpenAI-compatible chat endpoint. Supports `stream: true` for SSE.

```bash
curl http://localhost:8087/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "messages": [{"role": "user", "content": "List files in the current directory"}],
    "model": "qwen2.5"
  }'
```

### GET `/health`

Returns `{"status": "ok"}`.

## Architecture

```
agent-py/
├── agent.py          Main agent (tools, server, CLI, sidecar management)
├── requirements.txt  Pinned Python dependencies
├── .env              Local configuration (not committed)
└── .vscode/
    ├── launch.json   F5 debug configurations
    └── tasks.json    Build tasks
```

The Python agent starts the Rust `agent-shell` binary as a subprocess, communicating via HTTP on localhost. All tool calls flow through the LLM → agent framework → tool functions, with `delegate_to_agent_shell` forwarding complex tasks to the Rust backend.

## Pinned Versions

These versions are pinned to avoid known breakages:

- `agent-framework-core==1.0.0b260107`
- `agent-framework-ollama==1.0.0b260107`
- `opentelemetry-semantic-conventions-ai==0.4.13` (0.4.14 breaks with `SpanAttributes` error)
