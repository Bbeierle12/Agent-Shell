// ── Card Types ───────────────────────────────────────────────────────────
export enum CardType {
  CHAT = 'CHAT',
  SESSION = 'SESSION',
  NOTE = 'NOTE',
  ANALYTICS = 'ANALYTICS',
  TERMINAL = 'TERMINAL',
  SKILLS = 'SKILLS',
  CONTEXT = 'CONTEXT',
  PLUGINS = 'PLUGINS',
  ISLAND = 'ISLAND',
}

// ── Spatial ──────────────────────────────────────────────────────────────
export interface ViewportState { x: number; y: number; scale: number }

// ── Chat ─────────────────────────────────────────────────────────────────
export interface LocalChatMessage {
  role: 'user' | 'assistant'
  content: string
  toolCalls?: ToolCallItem[]
}

export interface ToolCallItem {
  name: string
  status: 'running' | 'done' | 'error'
  output?: string
  isError?: boolean
}

// ── Card ─────────────────────────────────────────────────────────────────
export interface CardSnapshot {
  x: number; y: number; width: number; height: number
  title?: string
  content?: string
  chatHistory?: LocalChatMessage[]
  sessionId?: string
  groupId?: string
  isCollapsed?: boolean
}

export interface CardData {
  id: string
  type: CardType
  x: number; y: number
  width: number; height: number
  title?: string
  zIndex: number
  // CHAT
  chatHistory?: LocalChatMessage[]
  sessionId?: string
  isLoading?: boolean
  // NOTE
  content?: string
  // ISLAND
  groupId?: string
  isCollapsed?: boolean
  // History
  history?: CardSnapshot[]
  historyIndex?: number
}

// ── Settings ─────────────────────────────────────────────────────────────
export interface AppSettings {
  theme: 'dark' | 'light'
  showGrid: boolean
  snapToGrid: boolean
  authToken: string
}

// ── API response shapes (mirror agent-server JSON) ────────────────────────
export interface ApiSession {
  id: string; name: string; message_count: number; updated_at: string
}

export interface ApiMessage {
  id: string; role: string; content: string
  tool_calls?: { id: string; name: string }[]
  tool_call_id?: string; timestamp: string
}

export interface ApiConfig {
  provider: { api_base: string; model: string; max_tokens: number; temperature: number; top_p: number; has_api_key: boolean }
  server: { host: string; port: number; cors: boolean; has_auth_token: boolean }
  session: { max_history: number; auto_save: boolean }
  sandbox: { mode: string; docker_image: string; timeout_secs: number }
  tools: string[]
}

export interface ApiSkill {
  name: string; description: string; tags: string[]; sub_skills: string[]; source?: string
}

export interface ApiPlugin {
  name: string; category: string; version?: string; enabled?: boolean
  description?: string; status?: string
}

export interface ApiPluginHealth {
  category: string; name: string; status: string
}

export interface ApiContext {
  project?: { name: string; project_type: string; path: string; git_remote?: string; git_branch?: string }
  git?: { branch?: string; remote?: string; is_dirty: boolean; head_short?: string; repo_root: string }
  environments: { name: string; env_type: string; version?: string; path: string }[]
}

export interface ApiAnalyticsSummary {
  total_sessions: number; active_days: number
  average_session_duration_secs?: number
  top_tools: [string, number][]
  deep_work_sessions: number
  today?: { sessions: number; messages: number; active_time: string; tool_calls: number; tool_errors: number }
}
