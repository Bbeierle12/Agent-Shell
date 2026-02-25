# Agent Shell — UI Integration Review

**Date:** 2026-02-25
**Scope:** Full review of `agent-ui` crate, `agent-server` routes, and `AppConfig` integration
**Files reviewed:** `crates/agent-ui/src/main.rs`, `crates/agent-ui/src/api.rs`, `crates/agent-ui/index.html`, `crates/agent-server/src/routes.rs`, `crates/agent-server/src/lib.rs`, `crates/agent-core/src/config.rs`

---

## Executive Summary

The web UI is a **minimal chat scaffold**. It has 5 Leptos components across a single 487-line file (`main.rs`) and a 292-line API client (`api.rs`). The backend exposes **13 API endpoints** spanning chat, sessions, plugins, skills, terminal (WebSocket PTY), context detection, and analytics — but **the UI only consumes 3 of them** (health, sessions, chat). The entire settings/configuration surface — provider selection, model parameters, sandbox mode, RAG settings, profiles, schedules — is invisible to the web user. There is no settings modal, no preferences panel, no confirmation dialogs, no toast notifications, no markdown rendering, no terminal emulator, no responsive layout, and almost no accessibility.

---

## 1. Missing Settings Modal (Critical)

**The #1 gap.** The backend has a rich `AppConfig` with 8 top-level sections:

| Config Section | Fields | UI Exposure |
|---|---|---|
| `provider` | api_base, model, api_key, max_tokens, temperature, top_p, failover | **None** |
| `providers` (multi-chain) | name, api_base, model, priority, timeout, retries, roles | **None** |
| `profiles` | description, model, api_base, system_prompt, max_tokens, temperature | **None** |
| `sandbox` | mode (docker/unsafe), docker_image, timeout, memory_limit, workspace_root | **None** |
| `rag` | qdrant_url, collection_name, embedding_model, chunk_size, top_k | **None** |
| `server` | host, port, auth_token, cors | **None** |
| `session` | history_dir, max_history, auto_save | **None** |
| `schedules` | name, cron, workspace, task type, skill, prompt, enabled | **None** |
| `system_prompt` | free-text | **None** |

The config is **only editable** via `~/.config/agent-shell/config.toml` or the CLI command `agent-shell config init`. A user in the web UI has **zero visibility** into which model they're talking to, what temperature is set, or which provider is active.

**What's needed:** A settings modal (gear icon in chat header or sidebar) with at minimum:
- Active model / provider display (read-only if not editable)
- Temperature / max_tokens sliders
- System prompt editor
- Session preferences (max_history, auto_save toggle)
- Connection info (API base, auth status)

---

## 2. Unused Backend API Endpoints

The UI calls only 4 of 13 available endpoints:

| Endpoint | Used by UI? | Purpose |
|---|---|---|
| `GET /health` | **Yes** | Connection status |
| `GET /v1/sessions` | **Yes** | Sidebar session list |
| `POST /v1/sessions` | **Yes** | New session button |
| `POST /v1/chat/completions` | **Yes** | Chat streaming |
| `GET /v1/plugins` | No | List loaded plugins |
| `GET /v1/plugins/health` | No | Plugin health status |
| `GET /v1/skills` | No | List available skills |
| `GET /v1/skills/search` | No | Search skills |
| `GET /v1/skills/{name}` | No | Read skill content |
| `GET /v1/terminal/shells` | No | List available shells |
| `GET /v1/terminal` (WS) | No | WebSocket PTY terminal |
| `GET /v1/context` | No | Project/git/env detection |
| `GET /v1/analytics/summary` | No | Usage analytics |
| `GET /v1/analytics/report` | No | Weekly/monthly reports |

**9 endpoints are completely invisible to the web user.** The backend has a full terminal emulator via WebSocket PTY, a skill system with search, plugin management, project context detection, and analytics — none of which appear in the UI.

---

## 3. Missing Core UI Components

| Component | Status | Notes |
|---|---|---|
| **Settings modal** | Missing | No gear icon, no preferences UI |
| **Confirmation dialogs** | Missing | Session deletion, dangerous actions have no confirmation |
| **Toast/notification system** | Missing | Errors silently appear inline in chat messages |
| **Dropdown menus** | Missing | No model selector, no profile switcher |
| **Tooltips** | Missing | Status dots, buttons have no hover explanations |
| **Loading spinners** | Partial | Only send button text changes ("..." vs "Send") |
| **Markdown rendering** | Missing | Assistant messages render as plain text (no code blocks, headers, lists) |
| **Code syntax highlighting** | Missing | Tool output and assistant code responses are unstyled |
| **Copy-to-clipboard** | Missing | No way to copy code blocks or tool output |
| **Session management** | Partial | Can create/switch sessions, cannot rename/delete/export |
| **Search** | Missing | No message search, no command palette |
| **Keyboard shortcuts** | Minimal | Only Enter-to-send; no Ctrl+K, Ctrl+N, Escape, etc. |
| **Terminal emulator** | Missing | Backend has full PTY support via WebSocket; UI has nothing |
| **Context/status bar** | Missing | No display of current model, token usage, git branch |
| **File upload** | Missing | No drag-and-drop or file attachment |

---

## 4. Accessibility Failures

The UI has near-zero accessibility support:

- **No ARIA attributes anywhere.** Buttons, session list, status indicators, tool cards — none have `aria-label`, `role`, `aria-live`, or `aria-expanded`.
- **No keyboard navigation.** Sidebar sessions can't be navigated with arrow keys. Tool cards can't be toggled with Enter/Space. No focus trap for future modals.
- **No focus management.** When a new session is created or a message is sent, focus isn't programmatically moved.
- **Status dot is visual-only.** The connected/disconnected/checking indicator is a colored `<span>` with no text alternative for screen readers.
- **No `aria-live` region.** New messages and streaming content aren't announced to assistive technology.
- **No heading hierarchy.** The sidebar uses `<h2>` for "Sessions" but the chat area uses class-based styling with no semantic headings.
- **Collapsed tool cards.** The toggle uses `on:click` with no `aria-expanded` or keyboard handler.

---

## 5. Responsive Design

**There is none.** Zero `@media` queries in `index.html`. The sidebar is a fixed `260px` / `min-width: 260px`, so on any viewport under ~600px the chat area becomes unusably narrow. On mobile the layout is completely broken.

Missing:
- Collapsible/drawer sidebar for mobile
- Hamburger menu trigger
- Touch-friendly tap targets (current buttons are small)
- Responsive font sizing
- Viewport-appropriate input area

---

## 6. Theme / Appearance

- **Dark mode only.** A single hardcoded dark theme (GitHub-dark inspired). No light mode. No system preference detection (`prefers-color-scheme`). No theme toggle.
- **CSS variables are well-structured** (`--bg`, `--surface`, `--border`, `--text`, `--accent`, etc.), so adding a light theme would be straightforward — the architecture is there, just unused.
- **All styles in `index.html`** `<style>` block (~290 lines). No external CSS file, no CSS modules, no scoped component styles.

---

## 7. Streaming & Message Handling Issues

- **No markdown rendering** (`main.rs:370-376`). Assistant text is dumped raw — code blocks with triple backticks, headers, lists all appear as plain text. This is arguably the most user-visible quality issue.
- **Tool output truncation is hard-coded** (`main.rs:428-429`). Output is sliced at 500 chars with `"..."` appended. No "Show more" button, no way to see the full output. The byte-level slice `&output[..500]` could also **panic on multi-byte UTF-8 characters**.
- **Empty trailing messages** (`main.rs:194-202`). There's cleanup logic to strip empty assistant messages after streaming, but during tool calls, an empty `AssistantMessage("")` is pushed after every `ToolResult` event (`main.rs:158`), creating potential flicker.
- **No auto-scroll.** The message list doesn't scroll to the bottom when new content arrives. Users must manually scroll during streaming.
- **No message history loading.** Clicking a session in the sidebar clears messages (`main.rs:213`) but never fetches the session's existing message history. Switching sessions always shows an empty chat.

---

## 8. State Management Gaps

- **No global error state.** Errors are crammed into the last `AssistantMessage` string. There's no error banner, no retry mechanism, no error boundary.
- **No loading state for sessions.** Creating a new session or switching has no loading indicator.
- **No optimistic updates.** The session list refetches fully after creation rather than optimistically inserting.
- **Session ID not sent with chat** (`api.rs:114-117`). The `stream_chat` function sends messages but doesn't include the `active_session` ID, even though the backend's `ChatRequest` has an optional `session_id` field. All messages go to whatever session the server's `SessionManager` considers "current."

---

## 9. API Client Issues

- **No auth token support** (`api.rs`). None of the API calls send an `Authorization` header. If the backend has `auth_token` configured, every request except `/health` will get a 401.
- **No request cancellation.** `AbortController` and `AbortSignal` are listed as `web-sys` features in `Cargo.toml` but never used. A streaming request can't be cancelled.
- **No retry logic.** Failed requests are reported once and discarded.
- **Hardcoded dev detection** (`api.rs:18-19`). Only checks for `127.0.0.1:8080` and `localhost:8080`. Doesn't handle other dev server ports or hostnames.

---

## 10. Security Concerns

- **No CSP headers.** The `index.html` has no Content-Security-Policy meta tag.
- **No auth token in UI requests.** As noted above, the auth middleware is bypassed from the UI side.
- **Inline styles only.** All CSS is in a `<style>` block, which is fine, but if CSP is added later, `style-src 'unsafe-inline'` will be required unless refactored.
- **No XSS protection layer for future markdown rendering.** Leptos escapes text by default (safe), but there's no sanitization layer if markdown rendering is added later.

---

## 11. What's Actually Well-Done

- **SSE streaming implementation** (`api.rs:108-226`) is solid — uses raw Fetch API with ReadableStream for POST-based SSE, handles buffer splitting correctly.
- **Tool card collapse/expand** (`main.rs:417-422`) is functional and clean.
- **Connection status indicator** is a nice touch with three visual states.
- **CSS variable architecture** is clean and extensible for future theming.
- **Backend API design** is well-structured with proper separation of routes, state, and middleware.
- **Auth middleware** on the server side is correctly implemented with public/protected route separation.

---

## Priority Recommendations

### P0 — Broken / Blocking

1. **Pass `session_id` in chat requests** — messages currently go to whichever session the server considers active, not the UI's `active_session`.
2. **Add auth token support to API client** — UI is locked out if `auth_token` is configured on the server.
3. **Add markdown rendering for assistant messages** — currently unreadable for any code or structured response.
4. **Fix potential UTF-8 panic in tool output truncation** — `&output[..500]` slices bytes, not characters.

### P1 — Basic Expected Functionality

5. Settings modal with model/provider info and key parameters
6. Auto-scroll messages during streaming
7. Load message history when switching sessions (requires new backend endpoint)
8. Toast/notification system for errors
9. Request cancellation (abort streaming on new message or navigation)

### P2 — Standard Quality

10. Terminal emulator component (backend PTY support is already built)
11. Analytics dashboard (backend endpoints are already built)
12. Responsive layout / mobile support
13. Light theme + system preference detection
14. Session rename/delete
15. Keyboard shortcuts (Ctrl+K command palette, Ctrl+N new session, Escape)

### P3 — Polish

16. Accessibility (ARIA attributes, keyboard navigation, screen reader support)
17. Code syntax highlighting
18. Copy-to-clipboard on code blocks and tool output
19. Plugin/skill browser UI
20. Context status bar showing current model, git branch, project info
