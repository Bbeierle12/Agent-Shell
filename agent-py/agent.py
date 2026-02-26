# Copyright (c) Microsoft. All rights reserved.
"""
Ollama Local Agentic Assistant

A fully agentic AI assistant powered by a local Ollama model with tools for:
- File reading & writing
- Directory listing
- Shell command execution (with optional Docker sandbox via agent-shell)
- Python code execution (sandboxed via agent-shell)
- Text search across files
- Web page fetching (with SSRF protection via agent-shell)

Integrates with the Rust agent-shell binary for sandboxed, production-grade
tool execution when available.

Usage:
  # HTTP server mode (default) - works with AI Toolkit Agent Inspector
  python agent.py

  # CLI mode for quick interactive testing
  python agent.py --cli
"""

import asyncio
import json
import os
import re
import signal
import subprocess
import sys
import time
from pathlib import Path
from typing import Annotated

import httpx
from agent_framework import ChatAgent, ai_function
from agent_framework.ollama import OllamaChatClient
from dotenv import load_dotenv

# Load .env (override=True for deployed environments)
load_dotenv(override=True)

# ──────────────────────────────────────────────
# Agent-Shell (Rust) Sidecar
# ──────────────────────────────────────────────

# Default: look for the Rust binary relative to this file (monorepo layout)
_DEFAULT_BINARY = str(Path(__file__).resolve().parent.parent / "target" / "release" / "agent-shell")
AGENT_SHELL_BINARY = os.getenv("AGENT_SHELL_BINARY", _DEFAULT_BINARY)
AGENT_SHELL_PORT = int(os.getenv("AGENT_SHELL_PORT", "8080"))
AGENT_SHELL_HOST = os.getenv("AGENT_SHELL_HOST", "127.0.0.1")
AGENT_SHELL_URL = f"http://{AGENT_SHELL_HOST}:{AGENT_SHELL_PORT}"

_agent_shell_process: subprocess.Popen | None = None


def start_agent_shell_sidecar() -> bool:
    """Start the Rust agent-shell HTTP server as a sidecar process.
    Returns True if the server is available (already running or just started).
    """
    global _agent_shell_process

    binary = Path(AGENT_SHELL_BINARY)
    if not binary.exists():
        print(f"[agent-shell] Binary not found at {binary}, running without Rust backend")
        return False

    # Check if already running
    try:
        resp = httpx.get(f"{AGENT_SHELL_URL}/health", timeout=2.0)
        if resp.status_code == 200:
            print(f"[agent-shell] Already running at {AGENT_SHELL_URL}")
            return True
    except Exception:
        pass

    # Start the sidecar
    try:
        _agent_shell_process = subprocess.Popen(
            [
                str(binary),
                "serve",
                "--host", AGENT_SHELL_HOST,
                "--port", str(AGENT_SHELL_PORT),
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        # Wait for it to be ready
        for _ in range(20):
            time.sleep(0.25)
            try:
                resp = httpx.get(f"{AGENT_SHELL_URL}/health", timeout=2.0)
                if resp.status_code == 200:
                    print(f"[agent-shell] Sidecar started on {AGENT_SHELL_URL} (PID {_agent_shell_process.pid})")
                    return True
            except Exception:
                continue
        print("[agent-shell] Sidecar started but health check not responding, continuing anyway")
        return True
    except Exception as e:
        print(f"[agent-shell] Failed to start sidecar: {e}")
        return False


def stop_agent_shell_sidecar() -> None:
    """Stop the Rust agent-shell sidecar if we started it."""
    global _agent_shell_process
    if _agent_shell_process is not None:
        print(f"[agent-shell] Stopping sidecar (PID {_agent_shell_process.pid})")
        _agent_shell_process.send_signal(signal.SIGTERM)
        try:
            _agent_shell_process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            _agent_shell_process.kill()
        _agent_shell_process = None


# Track whether agent-shell is available
_agent_shell_available = False

# ──────────────────────────────────────────────
# Agentic Tools
# ──────────────────────────────────────────────


@ai_function(approval_mode="never_require")
def read_file(
    file_path: Annotated[str, "Absolute or relative path to the file to read."],
) -> str:
    """Read and return the contents of a file."""
    path = Path(file_path).expanduser().resolve()
    if not path.exists():
        return f"Error: File not found: {path}"
    if not path.is_file():
        return f"Error: Path is not a file: {path}"
    try:
        content = path.read_text(encoding="utf-8", errors="replace")
        # Truncate very large files
        if len(content) > 50_000:
            return content[:50_000] + f"\n\n... [truncated, file is {len(content)} chars total]"
        return content
    except Exception as e:
        return f"Error reading file: {e}"


@ai_function(approval_mode="never_require")
def write_file(
    file_path: Annotated[str, "Absolute or relative path to the file to write."],
    content: Annotated[str, "The content to write to the file."],
) -> str:
    """Create or overwrite a file with the given content. Creates parent directories if needed."""
    path = Path(file_path).expanduser().resolve()
    try:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
        return f"Successfully wrote {len(content)} characters to {path}"
    except Exception as e:
        return f"Error writing file: {e}"


@ai_function(approval_mode="never_require")
def list_directory(
    directory_path: Annotated[str, "Path to the directory to list. Defaults to current directory."] = ".",
) -> str:
    """List contents of a directory with file types and sizes."""
    path = Path(directory_path).expanduser().resolve()
    if not path.exists():
        return f"Error: Directory not found: {path}"
    if not path.is_dir():
        return f"Error: Path is not a directory: {path}"
    try:
        entries = sorted(path.iterdir(), key=lambda p: (not p.is_dir(), p.name.lower()))
        lines = []
        for entry in entries:
            if entry.name.startswith("."):
                continue  # skip hidden files by default
            if entry.is_dir():
                lines.append(f"  [DIR]  {entry.name}/")
            else:
                size = entry.stat().st_size
                if size < 1024:
                    size_str = f"{size} B"
                elif size < 1024 * 1024:
                    size_str = f"{size / 1024:.1f} KB"
                else:
                    size_str = f"{size / (1024 * 1024):.1f} MB"
                lines.append(f"  [FILE] {entry.name}  ({size_str})")
        if not lines:
            return f"Directory is empty: {path}"
        return f"Contents of {path}:\n" + "\n".join(lines)
    except Exception as e:
        return f"Error listing directory: {e}"


@ai_function(approval_mode="never_require")
def run_shell_command(
    command: Annotated[str, "The shell command to execute."],
    working_directory: Annotated[str, "Working directory for the command. Defaults to current directory."] = ".",
) -> str:
    """Execute a shell command and return its output. Use for running scripts, git, build tools, etc."""
    cwd = Path(working_directory).expanduser().resolve()
    try:
        result = subprocess.run(
            command,
            shell=True,
            cwd=str(cwd),
            capture_output=True,
            text=True,
            timeout=60,
        )
        output = ""
        if result.stdout:
            output += result.stdout
        if result.stderr:
            output += ("\n--- stderr ---\n" + result.stderr) if output else result.stderr
        if result.returncode != 0:
            output += f"\n[Exit code: {result.returncode}]"
        if not output.strip():
            output = f"[Command completed with exit code {result.returncode}]"
        # Truncate very long output
        if len(output) > 30_000:
            output = output[:30_000] + "\n\n... [output truncated]"
        return output
    except subprocess.TimeoutExpired:
        return "Error: Command timed out after 60 seconds."
    except Exception as e:
        return f"Error running command: {e}"


@ai_function(approval_mode="never_require")
def run_python_code(
    code: Annotated[str, "The Python code to execute."],
) -> str:
    """Execute Python code and return its output. Code runs in a local subprocess."""
    try:
        result = subprocess.run(
            [sys.executable, "-c", code],
            capture_output=True,
            text=True,
            timeout=60,
        )
        output = ""
        if result.stdout:
            output += result.stdout
        if result.stderr:
            output += ("\n--- stderr ---\n" + result.stderr) if output else result.stderr
        if result.returncode != 0:
            output += f"\n[Exit code: {result.returncode}]"
        if not output.strip():
            output = f"[Code executed with exit code {result.returncode}]"
        if len(output) > 30_000:
            output = output[:30_000] + "\n\n... [output truncated]"
        return output
    except subprocess.TimeoutExpired:
        return "Error: Code execution timed out after 60 seconds."
    except Exception as e:
        return f"Error running Python code: {e}"


@ai_function(approval_mode="never_require")
async def delegate_to_agent_shell(
    task: Annotated[str, "A natural language task to delegate to the Rust agent-shell for sandboxed execution. Use this for complex multi-step tasks requiring secure shell/python/file operations."],
) -> str:
    """Delegate a complex task to the Rust agent-shell backend, which provides
    Docker-sandboxed code execution, SSRF-protected web fetching, and
    workspace-restricted file operations. Only available when agent-shell is running."""
    if not _agent_shell_available:
        return "Error: agent-shell backend is not available. Use the direct tools instead."
    try:
        async with httpx.AsyncClient(timeout=120.0) as client:
            resp = await client.post(
                f"{AGENT_SHELL_URL}/v1/chat/completions",
                json={
                    "messages": [{"role": "user", "content": task}],
                    "stream": False,
                },
            )
            if resp.status_code != 200:
                return f"agent-shell returned status {resp.status_code}: {resp.text}"
            data = resp.json()
            choices = data.get("choices", [])
            if choices:
                return choices[0].get("message", {}).get("content", "No response from agent-shell")
            return "No response from agent-shell"
    except Exception as e:
        return f"Error communicating with agent-shell: {e}"


@ai_function(approval_mode="never_require")
def search_in_files(
    pattern: Annotated[str, "Text or pattern to search for."],
    directory: Annotated[str, "Directory to search in. Defaults to current directory."] = ".",
    file_extension: Annotated[str, "Optional file extension filter, e.g. '.py', '.ts'. Empty means all files."] = "",
) -> str:
    """Search for a text pattern across files in a directory (recursive). Similar to grep."""
    search_dir = Path(directory).expanduser().resolve()
    if not search_dir.exists():
        return f"Error: Directory not found: {search_dir}"

    matches = []
    try:
        glob_pattern = f"**/*{file_extension}" if file_extension else "**/*"
        for filepath in search_dir.glob(glob_pattern):
            if not filepath.is_file():
                continue
            # Skip binary files, hidden dirs, common non-text dirs
            rel = filepath.relative_to(search_dir)
            parts = rel.parts
            if any(p.startswith(".") or p in ("node_modules", "__pycache__", ".git", "venv", ".venv") for p in parts):
                continue
            try:
                text = filepath.read_text(encoding="utf-8", errors="ignore")
                for i, line in enumerate(text.splitlines(), 1):
                    if pattern.lower() in line.lower():
                        matches.append(f"{rel}:{i}: {line.strip()}")
                        if len(matches) >= 100:
                            matches.append("... [results truncated at 100 matches]")
                            return "\n".join(matches)
            except (UnicodeDecodeError, PermissionError):
                continue
    except Exception as e:
        return f"Error searching files: {e}"

    if not matches:
        return f"No matches found for '{pattern}' in {search_dir}"
    return "\n".join(matches)


@ai_function(approval_mode="never_require")
async def fetch_webpage(
    url: Annotated[str, "The URL to fetch content from."],
) -> str:
    """Fetch the text content of a web page or API endpoint."""
    try:
        async with httpx.AsyncClient(follow_redirects=True, timeout=30.0) as client:
            response = await client.get(url, headers={"User-Agent": "OllamaAgent/1.0"})
            content_type = response.headers.get("content-type", "")
            if "json" in content_type:
                text = response.text
            elif "html" in content_type:
                # Strip HTML tags for readability
                text = re.sub(r"<script[^>]*>.*?</script>", "", response.text, flags=re.DOTALL)
                text = re.sub(r"<style[^>]*>.*?</style>", "", text, flags=re.DOTALL)
                text = re.sub(r"<[^>]+>", " ", text)
                text = re.sub(r"\s+", " ", text).strip()
            else:
                text = response.text

            if len(text) > 30_000:
                text = text[:30_000] + "\n\n... [content truncated]"
            return f"[Status {response.status_code}]\n{text}"
    except Exception as e:
        return f"Error fetching URL: {e}"


# ──────────────────────────────────────────────
# Agent Setup
# ──────────────────────────────────────────────

AGENT_INSTRUCTIONS = """\
You are a powerful local AI coding assistant running on the user's machine via Ollama.
You have full agentic capabilities through the tools available to you.

## Your Capabilities
- **Read files**: Use `read_file` to examine source code, configs, logs, etc.
- **Write files**: Use `write_file` to create or modify files.
- **List directories**: Use `list_directory` to explore project structure.
- **Run commands**: Use `run_shell_command` to execute git, build tools, scripts, tests, etc.
- **Run Python**: Use `run_python_code` to execute Python code snippets.
- **Search code**: Use `search_in_files` to find patterns across a codebase.
- **Fetch web content**: Use `fetch_webpage` to retrieve documentation, API responses, etc.
- **Delegate complex tasks**: Use `delegate_to_agent_shell` to send complex multi-step tasks \
to the Rust agent-shell backend which runs with Docker-sandboxed execution and SSRF protection.

## Guidelines
- When asked to work on code, first explore the project structure and relevant files.
- Before making changes, read the existing code to understand context.
- After writing files, verify the changes if appropriate (e.g., run linters or tests).
- For untrusted or risky operations, prefer `delegate_to_agent_shell` for sandboxed execution.
- Be thorough: use multiple tools in sequence to accomplish complex tasks.
- Explain what you're doing and why at each step.
- If a command fails, analyze the error and try to fix it.
"""

ALL_TOOLS = [
    read_file,
    write_file,
    list_directory,
    run_shell_command,
    run_python_code,
    search_in_files,
    fetch_webpage,
    delegate_to_agent_shell,
]


def create_agent() -> ChatAgent:
    """Create the Ollama-powered agentic assistant."""
    client = OllamaChatClient()
    return ChatAgent(
        chat_client=client,
        instructions=AGENT_INSTRUCTIONS,
        tools=ALL_TOOLS,
    )


# ──────────────────────────────────────────────
# Entrypoints: HTTP Server (default) & CLI
# ──────────────────────────────────────────────


async def run_cli() -> None:
    """Interactive CLI mode for quick testing."""
    global _agent_shell_available
    _agent_shell_available = start_agent_shell_sidecar()

    print("=" * 60)
    print("  Ollama Local Agent — CLI Mode")
    print(f"  Model: {os.getenv('OLLAMA_MODEL_ID', 'default')}")
    print(f"  Agent-Shell: {'connected' if _agent_shell_available else 'not available'}")
    print("  Type 'exit' or 'quit' to stop.")
    print("=" * 60)

    try:
        async with create_agent() as agent:
            thread = agent.get_new_thread()
            while True:
                try:
                    user_input = input("\nYou: ").strip()
                except (EOFError, KeyboardInterrupt):
                    print("\nGoodbye!")
                    break
                if not user_input or user_input.lower() in ("exit", "quit"):
                    print("Goodbye!")
                    break

                print("Agent: ", end="", flush=True)
                async for chunk in agent.run_stream(user_input, thread=thread):
                    if chunk.text:
                        print(chunk.text, end="", flush=True)
                print()
    finally:
        stop_agent_shell_sidecar()


async def run_server() -> None:
    """HTTP server mode with OpenAI-compatible chat API + SSE streaming.
    Works with AI Toolkit Agent Inspector and any OpenAI-compatible client.
    """
    import uuid

    import uvicorn
    from fastapi import FastAPI
    from fastapi.middleware.cors import CORSMiddleware
    from fastapi.responses import StreamingResponse
    from pydantic import BaseModel

    global _agent_shell_available
    _agent_shell_available = start_agent_shell_sidecar()

    app = FastAPI(title="Ollama Local Agent")
    app.add_middleware(
        CORSMiddleware,
        allow_origins=["*"],
        allow_methods=["*"],
        allow_headers=["*"],
    )

    # Shared agent + thread per session (with simple LRU eviction)
    agent_instance = create_agent()
    threads: dict[str, object] = {}
    MAX_SESSIONS = 100

    class ChatMessageInput(BaseModel):
        role: str
        content: str

    class ChatRequest(BaseModel):
        messages: list[ChatMessageInput]
        stream: bool = False
        session_id: str | None = None

    @app.get("/health")
    async def health():
        return {"status": "ok", "agent_shell": _agent_shell_available}

    @app.post("/v1/chat/completions")
    async def chat_completions(req: ChatRequest):
        session_id = req.session_id or "default"
        if session_id not in threads:
            # Evict oldest sessions if at capacity
            if len(threads) >= MAX_SESSIONS:
                oldest = next(iter(threads))
                del threads[oldest]
            threads[session_id] = agent_instance.get_new_thread()
        thread = threads[session_id]

        user_msg = req.messages[-1].content if req.messages else ""

        if req.stream:
            async def event_stream():
                async for chunk in agent_instance.run_stream(user_msg, thread=thread):
                    if chunk.text:
                        data = json.dumps({
                            "choices": [{"delta": {"content": chunk.text}, "index": 0}]
                        })
                        yield f"data: {data}\n\n"
                yield "data: [DONE]\n\n"

            return StreamingResponse(event_stream(), media_type="text/event-stream")
        else:
            result = await agent_instance.run(user_msg, thread=thread)
            return {
                "id": str(uuid.uuid4()),
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": str(result)},
                    "finish_reason": "stop",
                }],
            }

    server_port = int(os.getenv("AGENT_SERVER_PORT", "8087"))
    print(f"Starting Ollama Agent HTTP Server on port {server_port}...")
    print(f"  Agent-Shell backend: {'connected' if _agent_shell_available else 'not available'}")
    print(f"  Model: {os.getenv('OLLAMA_MODEL_ID', 'default')}")

    config = uvicorn.Config(app, host="0.0.0.0", port=server_port, log_level="info")
    server = uvicorn.Server(config)
    try:
        await server.serve()
    finally:
        stop_agent_shell_sidecar()


def main() -> None:
    if "--cli" in sys.argv:
        asyncio.run(run_cli())
    else:
        asyncio.run(run_server())


if __name__ == "__main__":
    main()
