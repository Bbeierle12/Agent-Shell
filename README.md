# agent-shell

A model-agnostic AI agent shell with built-in tool execution, session management, and sandboxed code execution. Works with any OpenAI-compatible API endpoint (local or remote).

## Quickstart

```bash
# Build
cargo build --release

# Run the interactive REPL (uses default config)
./target/release/agent-shell

# Initialize a config file
./target/release/agent-shell config init

# Start the HTTP server
./target/release/agent-shell serve
```

## Configuration

Configuration is stored at `~/.config/agent-shell/config.toml`. Key sections:

```toml
[provider]
api_base = "http://localhost:11434/v1"
model = "glm-4.7-swift"
# api_key = "your-key"

[sandbox]
mode = "docker"                    # "docker" (default, isolated) or "unsafe" (direct)
docker_image = "python:3.12-slim"
timeout_secs = 30
# workspace_root = "/home/user/projects"   # restricts file tools to this directory

[server]
host = "127.0.0.1"
port = 8080
# auth_token = "your-secret-token"  # bearer token for HTTP API auth
cors = true
```

## Security

- **Sandbox mode defaults to `docker`** for isolated code execution. Only set `mode = "unsafe"` if you understand the risks.
- **`workspace_root`**: When set, file read/write/list tools are restricted to paths under this directory. Symlink traversal is blocked via canonicalization.
- **`auth_token`**: Always set this when exposing the HTTP server. Without it, anyone who can reach the server can execute tools.
- **SSRF protection**: The `web_fetch` tool blocks requests to localhost, private IPs, link-local addresses, and cloud metadata endpoints.

## Built-in Tools

| Tool | Description |
|------|-------------|
| `shell_exec` | Execute shell commands (sandboxed via Docker or direct) |
| `python_exec` | Execute Python code (sandboxed via Docker or direct) |
| `file_read` | Read file contents with optional line range |
| `file_write` | Write or append to files |
| `file_list` | List directory contents (flat or recursive) |
| `web_fetch` | Fetch web pages by URL (with SSRF protection) |

## Architecture

```
agent-shell (binary)
├── src/main.rs          CLI entry point (clap)
├── src/repl.rs          Interactive REPL
│
├── crates/agent-core    Core library
│   ├── agent_loop.rs    LLM orchestration with tool calling
│   ├── config.rs        TOML configuration
│   ├── session.rs       Session persistence
│   ├── tool_registry.rs Tool trait and registry
│   ├── types.rs         Message, ToolCall, AgentEvent types
│   └── error.rs         Error types
│
├── crates/agent-tools   Built-in tool implementations
│   ├── file_ops.rs      File read/write/list with workspace validation
│   ├── shell_exec.rs    Shell command execution
│   ├── python_exec.rs   Python code execution
│   ├── web_fetch.rs     HTTP fetching with SSRF protection
│   └── sandbox.rs       Docker/unsafe execution backend
│
└── crates/agent-server  HTTP server mode
    ├── lib.rs           Router, auth middleware, CORS
    ├── routes.rs        REST + SSE streaming endpoints
    └── state.rs         Shared application state
```

## License

MIT
