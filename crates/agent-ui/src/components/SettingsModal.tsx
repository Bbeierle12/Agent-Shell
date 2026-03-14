import { useState, useEffect } from 'react'
import { Sun, Moon, Grid, Magnet, Key, Trash2, Check, RefreshCw } from 'lucide-react'
import { AppSettings, ApiConfig } from '../types'
import { setAuthToken, listModels, updateProvider, type OllamaModel } from '../services/api'

interface Props {
  settings: AppSettings
  config: ApiConfig | null
  onUpdate: (updates: Partial<AppSettings>) => void
  onClose: () => void
  onResetCanvas: () => void
  onConfigChange?: (config: ApiConfig) => void
}

export function SettingsModal({ settings, config, onUpdate, onClose, onResetCanvas, onConfigChange }: Props) {
  const [token, setToken] = useState(settings.authToken)
  const [tokenSaved, setTokenSaved] = useState(false)
  const [models, setModels] = useState<OllamaModel[]>([])
  const [loadingModels, setLoadingModels] = useState(false)
  const [switching, setSwitching] = useState(false)
  const [switchStatus, setSwitchStatus] = useState<'idle' | 'success' | 'error'>('idle')

  useEffect(() => { setToken(settings.authToken) }, [settings.authToken])

  // Load available models when modal opens
  useEffect(() => {
    setLoadingModels(true)
    listModels()
      .then(setModels)
      .catch(() => setModels([]))
      .finally(() => setLoadingModels(false))
  }, [])

  const refreshModels = () => {
    setLoadingModels(true)
    listModels()
      .then(setModels)
      .catch(() => setModels([]))
      .finally(() => setLoadingModels(false))
  }

  const switchModel = async (modelName: string) => {
    if (!config || modelName === config.provider.model) return
    setSwitching(true)
    setSwitchStatus('idle')
    try {
      const updated = await updateProvider({ model: modelName })
      setSwitchStatus('success')
      onConfigChange?.(updated)
      setTimeout(() => setSwitchStatus('idle'), 2000)
    } catch {
      setSwitchStatus('error')
      setTimeout(() => setSwitchStatus('idle'), 2000)
    } finally {
      setSwitching(false)
    }
  }

  const saveToken = () => {
    setAuthToken(token)
    onUpdate({ authToken: token })
    setTokenSaved(true)
    setTimeout(() => setTokenSaved(false), 1800)
  }

  const formatSize = (bytes: number) => {
    const gb = bytes / (1024 * 1024 * 1024)
    return gb >= 1 ? `${gb.toFixed(1)} GB` : `${(bytes / (1024 * 1024)).toFixed(0)} MB`
  }

  const Toggle = ({ on, onToggle }: { on: boolean; onToggle: () => void }) => (
    <button
      onClick={onToggle}
      style={{
        width: 44, height: 24,
        borderRadius: 12,
        border: 'none',
        background: on ? 'var(--success)' : 'var(--border)',
        position: 'relative',
        cursor: 'pointer',
        transition: 'background 0.2s',
        flexShrink: 0,
      }}
    >
      <span style={{
        position: 'absolute',
        top: 2, left: on ? 22 : 2,
        width: 20, height: 20,
        borderRadius: '50%',
        background: '#fff',
        transition: 'left 0.2s',
        boxShadow: '0 1px 3px rgba(0,0,0,0.3)',
      }} />
    </button>
  )

  const Row = ({ label, children }: { label: React.ReactNode; children: React.ReactNode }) => (
    <div className="setting-row">
      <span className="setting-label">{label}</span>
      {children}
    </div>
  )

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal-box" onClick={e => e.stopPropagation()}>
        <div className="modal-hd">
          <h2>Settings</h2>
          <button className="modal-close" onClick={onClose}>×</button>
        </div>
        <div className="modal-bd">

          {/* Model Selector */}
          <div className="settings-section">
            <h3 style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
              Model
              <button
                onClick={refreshModels}
                disabled={loadingModels}
                style={{
                  background: 'none', border: '1px solid var(--border)', color: 'var(--text-muted)',
                  padding: '2px 6px', borderRadius: 4, cursor: 'pointer', fontSize: 11,
                  display: 'flex', alignItems: 'center', gap: 4, textTransform: 'none', letterSpacing: 0, fontWeight: 400,
                }}
              >
                <RefreshCw size={10} style={{ animation: loadingModels ? 'spin 1s linear infinite' : 'none' }} />
                Refresh
              </button>
            </h3>
            {config && (
              <div style={{ marginBottom: 8, fontSize: 12, color: 'var(--text-muted)' }}>
                Current: <span style={{ color: 'var(--accent)', fontFamily: 'monospace' }}>{config.provider.model}</span>
                {switchStatus === 'success' && <span style={{ color: 'var(--success)', marginLeft: 8 }}>Switched!</span>}
                {switchStatus === 'error' && <span style={{ color: 'var(--error)', marginLeft: 8 }}>Failed to switch</span>}
              </div>
            )}
            <div style={{
              background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 8,
              maxHeight: 200, overflowY: 'auto',
            }}>
              {loadingModels ? (
                <div style={{ padding: '12px', textAlign: 'center', fontSize: 12, color: 'var(--text-muted)' }}>
                  Loading models...
                </div>
              ) : models.length === 0 ? (
                <div style={{ padding: '12px', textAlign: 'center', fontSize: 12, color: 'var(--text-muted)' }}>
                  No models found. Is Ollama running?
                </div>
              ) : (
                models.map(m => {
                  const isActive = config?.provider.model === m.name
                  return (
                    <button
                      key={m.name}
                      disabled={switching || isActive}
                      onClick={() => switchModel(m.name)}
                      style={{
                        display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                        width: '100%', padding: '8px 12px',
                        borderBottom: '1px solid var(--border)',
                        background: isActive ? 'rgba(88, 166, 255, 0.1)' : 'transparent',
                        border: 'none', borderBottomStyle: 'solid', borderBottomWidth: 1, borderBottomColor: 'var(--border)',
                        cursor: isActive ? 'default' : 'pointer',
                        textAlign: 'left', fontSize: 13, color: 'var(--text)',
                      }}
                    >
                      <div>
                        <span style={{
                          fontFamily: 'monospace', fontSize: 12,
                          color: isActive ? 'var(--accent)' : 'var(--text)',
                          fontWeight: isActive ? 600 : 400,
                        }}>
                          {m.name}
                        </span>
                        {isActive && <span style={{ fontSize: 10, color: 'var(--success)', marginLeft: 6 }}>active</span>}
                      </div>
                      <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>
                        {formatSize(m.size)}
                      </span>
                    </button>
                  )
                })
              )}
            </div>
          </div>

          {/* Appearance */}
          <div className="settings-section">
            <h3>Appearance</h3>
            <Row label="Theme">
              <div style={{ display: 'flex', gap: 4, background: 'var(--bg)', borderRadius: 8, padding: 3 }}>
                <button
                  onClick={() => onUpdate({ theme: 'dark' })}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 5,
                    padding: '4px 12px', borderRadius: 6, border: 'none', cursor: 'pointer', fontSize: 12,
                    background: settings.theme === 'dark' ? 'var(--surface)' : 'transparent',
                    color: settings.theme === 'dark' ? 'var(--text)' : 'var(--text-muted)',
                    boxShadow: settings.theme === 'dark' ? '0 1px 3px rgba(0,0,0,0.3)' : 'none',
                  }}
                >
                  <Moon size={12} /> Dark
                </button>
                <button
                  onClick={() => onUpdate({ theme: 'light' })}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 5,
                    padding: '4px 12px', borderRadius: 6, border: 'none', cursor: 'pointer', fontSize: 12,
                    background: settings.theme === 'light' ? 'var(--surface)' : 'transparent',
                    color: settings.theme === 'light' ? 'var(--text)' : 'var(--text-muted)',
                    boxShadow: settings.theme === 'light' ? '0 1px 3px rgba(0,0,0,0.15)' : 'none',
                  }}
                >
                  <Sun size={12} /> Light
                </button>
              </div>
            </Row>
          </div>

          {/* Canvas */}
          <div className="settings-section">
            <h3>Canvas</h3>
            <Row label={<span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Grid size={13} /> Show Grid</span>}>
              <Toggle on={settings.showGrid} onToggle={() => onUpdate({ showGrid: !settings.showGrid })} />
            </Row>
            <Row label={<span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Magnet size={13} /> Snap to Grid</span>}>
              <Toggle on={settings.snapToGrid} onToggle={() => onUpdate({ snapToGrid: !settings.snapToGrid })} />
            </Row>
          </div>

          {/* Auth */}
          <div className="settings-section">
            <h3>Agent Server</h3>
            <Row label={<span style={{ display: 'flex', alignItems: 'center', gap: 6 }}><Key size={13} /> Auth Token</span>}>
              <div style={{ display: 'flex', gap: 6, alignItems: 'center' }}>
                <input
                  className="setting-input"
                  type="password"
                  value={token}
                  onChange={e => { setToken(e.target.value); setTokenSaved(false) }}
                  onKeyDown={e => e.key === 'Enter' && saveToken()}
                  placeholder="Bearer token…"
                />
                <button
                  className="toggle-btn"
                  onClick={saveToken}
                  style={tokenSaved ? { borderColor: 'var(--success)', color: 'var(--success)', background: 'rgba(63,185,80,0.08)' } : {}}
                >
                  {tokenSaved ? <Check size={13} /> : 'Save'}
                </button>
              </div>
            </Row>
          </div>

          {/* Server info (read-only) */}
          {config && (
            <div className="settings-section">
              <h3>Server Info</h3>
              <div style={{ background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden' }}>
                {[
                  ['Endpoint', config.provider.api_base],
                  ['Max Tokens', String(config.provider.max_tokens)],
                  ['Temperature', config.provider.temperature.toFixed(1)],
                  ['API Key', config.provider.has_api_key ? 'configured' : 'not set'],
                  ['Auth Token', config.server.has_auth_token ? 'enabled' : 'disabled'],
                  ['Context Window', `${config.session.max_history} messages`],
                  ['Sandbox', config.sandbox.mode],
                ].map(([label, value]) => (
                  <div key={label} style={{ display: 'flex', justifyContent: 'space-between', padding: '5px 10px', borderBottom: '1px solid var(--border)', fontSize: 12 }}>
                    <span style={{ color: 'var(--text-muted)' }}>{label}</span>
                    <span style={{
                      fontFamily: 'monospace', color: 'var(--text)',
                      maxWidth: 240, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                    }}>{value}</span>
                  </div>
                ))}
                <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', padding: '5px 10px', fontSize: 12 }}>
                  <span style={{ color: 'var(--text-muted)' }}>Tools</span>
                  <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', justifyContent: 'flex-end', maxWidth: 260 }}>
                    {config.tools.map(t => <span key={t} className="badge badge-blue">{t}</span>)}
                  </div>
                </div>
              </div>
            </div>
          )}

          {/* Danger zone */}
          <div className="settings-section">
            <h3>Danger Zone</h3>
            <Row label="Clear all cards from canvas">
              <button
                style={{ display: 'flex', alignItems: 'center', gap: 5, background: 'none', border: '1px solid var(--error)', color: 'var(--error)', padding: '5px 12px', borderRadius: 7, cursor: 'pointer', fontSize: 12 }}
                onClick={() => { onResetCanvas(); onClose() }}
              >
                <Trash2 size={13} /> Clear Canvas
              </button>
            </Row>
          </div>

        </div>
      </div>
    </div>
  )
}
