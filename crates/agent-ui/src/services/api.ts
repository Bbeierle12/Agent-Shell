import type {
  ApiSession, ApiMessage, ApiConfig, ApiSkill,
  ApiPlugin, ApiPluginHealth, ApiContext, ApiAnalyticsSummary,
} from '../types'

// ── Auth token stored in localStorage ─────────────────────────────────
const TOKEN_KEY = 'agent_shell_token'

export function getAuthToken(): string {
  return localStorage.getItem(TOKEN_KEY) ?? ''
}

export function setAuthToken(token: string): void {
  localStorage.setItem(TOKEN_KEY, token)
}

function authHeaders(): Record<string, string> {
  const token = getAuthToken()
  const h: Record<string, string> = { 'Content-Type': 'application/json' }
  if (token) h['Authorization'] = `Bearer ${token}`
  return h
}

async function get<T>(path: string): Promise<T> {
  const res = await fetch(path, { headers: authHeaders() })
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return res.json() as Promise<T>
}

// ── Health ─────────────────────────────────────────────────────────────
export async function healthCheck(): Promise<void> {
  const res = await fetch('/health')
  if (!res.ok) throw new Error('unhealthy')
}

// ── Config ─────────────────────────────────────────────────────────────
export function getConfig(): Promise<ApiConfig> {
  return get<ApiConfig>('/v1/config')
}

// ── Sessions ───────────────────────────────────────────────────────────
export function listSessions(): Promise<ApiSession[]> {
  return get<ApiSession[]>('/v1/sessions')
}

export async function createSession(name: string): Promise<{ id: string; name: string }> {
  const res = await fetch('/v1/sessions', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ name }),
  })
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return res.json() as Promise<{ id: string; name: string }>
}

export function getSessionMessages(id: string): Promise<ApiMessage[]> {
  return get<ApiMessage[]>(`/v1/sessions/${id}/messages`)
}

// ── Chat (SSE streaming) ───────────────────────────────────────────────
type StreamEvent =
  | { type: 'token'; content: string }
  | { type: 'tool_start'; name: string }
  | { type: 'tool_result'; content: string; isError: boolean }
  | { type: 'done' }
  | { type: 'error'; message: string }

export async function streamChat(
  messages: { role: string; content: string }[],
  onEvent: (e: StreamEvent) => void,
): Promise<void> {
  const res = await fetch('/v1/chat/completions', {
    method: 'POST',
    headers: authHeaders(),
    body: JSON.stringify({ messages, stream: true }),
  })

  if (!res.ok || !res.body) {
    onEvent({ type: 'error', message: `${res.status} ${res.statusText}` })
    return
  }

  const reader = res.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  let currentEvent = ''

  while (true) {
    const { done, value } = await reader.read()
    if (done) break
    buffer += decoder.decode(value, { stream: true })

    const lines = buffer.split('\n')
    buffer = lines.pop() ?? ''

    for (const line of lines) {
      if (line.startsWith('event: ')) {
        currentEvent = line.slice(7).trim()
      } else if (line.startsWith('data: ')) {
        const data = line.slice(6).trim()
        if (data === '[DONE]') {
          onEvent({ type: 'done' })
          currentEvent = ''
          continue
        }
        try {
          const parsed = JSON.parse(data)
          if (currentEvent === 'tool_call') {
            onEvent({ type: 'tool_start', name: parsed.tool as string })
          } else if (currentEvent === 'tool_result') {
            onEvent({
              type: 'tool_result',
              content: parsed.content as string,
              isError: parsed.is_error as boolean,
            })
          } else if (currentEvent === 'error') {
            onEvent({ type: 'error', message: data })
          } else {
            // Default: content chunk — {"choices":[{"delta":{"content":"..."}}]}
            const token = parsed?.choices?.[0]?.delta?.content as string | undefined
            if (token != null) onEvent({ type: 'token', content: token })
          }
        } catch {
          if (currentEvent === 'error') onEvent({ type: 'error', message: data })
        }
        if (!line.startsWith('event: ')) currentEvent = ''
      } else if (line === '') {
        // blank line ends SSE event block
        currentEvent = ''
      }
    }
  }
}

// ── Skills ─────────────────────────────────────────────────────────────
export function listSkills(): Promise<ApiSkill[]> {
  return get<ApiSkill[]>('/v1/skills')
}

export function searchSkills(q: string, limit = 20): Promise<ApiSkill[]> {
  return get<ApiSkill[]>(`/v1/skills/search?q=${encodeURIComponent(q)}&limit=${limit}`)
}

export async function getSkillContent(name: string): Promise<string> {
  const res = await fetch(`/v1/skills/${encodeURIComponent(name)}`, { headers: authHeaders() })
  if (!res.ok) throw new Error(`${res.status}`)
  return res.json() as Promise<string>
}

// ── Analytics ──────────────────────────────────────────────────────────
export function getAnalyticsSummary(): Promise<ApiAnalyticsSummary> {
  return get<ApiAnalyticsSummary>('/v1/analytics/summary')
}

export async function getAnalyticsReport(period: 'week' | 'month'): Promise<string> {
  const res = await fetch(`/v1/analytics/report?period=${period}`, { headers: authHeaders() })
  if (!res.ok) throw new Error(`${res.status}`)
  return res.text()
}

// ── Context ────────────────────────────────────────────────────────────
export function getContext(directory?: string): Promise<ApiContext> {
  const url = directory
    ? `/v1/context?directory=${encodeURIComponent(directory)}`
    : '/v1/context'
  return get<ApiContext>(url)
}

// ── Plugins ────────────────────────────────────────────────────────────
export function listPlugins(): Promise<ApiPlugin[]> {
  return get<ApiPlugin[]>('/v1/plugins')
}

export function getPluginHealth(): Promise<ApiPluginHealth[]> {
  return get<ApiPluginHealth[]>('/v1/plugins/health')
}

// ── Terminal WebSocket ─────────────────────────────────────────────────
export function createTerminalSocket(): WebSocket {
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return new WebSocket(`${proto}//${window.location.host}/v1/terminal`)
}
