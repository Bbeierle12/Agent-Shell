import { useState, useEffect } from 'react'
import { ApiPlugin, ApiPluginHealth } from '../../types'
import { listPlugins, getPluginHealth } from '../../services/api'

export function PluginsCard() {
  const [plugins, setPlugins] = useState<ApiPlugin[]>([])
  const [health, setHealth] = useState<ApiPluginHealth[]>([])
  const [tab, setTab] = useState<'plugins' | 'health'>('plugins')
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    Promise.all([listPlugins(), getPluginHealth()])
      .then(([p, h]) => { setPlugins(p); setHealth(h) })
      .catch(() => {})
      .finally(() => setLoading(false))
  }, [])

  const statusBadge = (s: string) => {
    const lc = s.toLowerCase()
    const cls = lc === 'ok' || lc === 'available' ? 'badge-green' : lc.includes('error') ? 'badge-red' : 'badge-gray'
    return <span className={`badge ${cls}`}>{s}</span>
  }

  return (
    <div className="card-inner" style={{ display: 'flex', flexDirection: 'column' }}>
      <div className="tab-row">
        <button className={`tab-btn${tab === 'plugins' ? ' active' : ''}`} onClick={() => setTab('plugins')}>Plugins</button>
        <button className={`tab-btn${tab === 'health' ? ' active' : ''}`} onClick={() => setTab('health')}>Health</button>
      </div>

      {loading && <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Loading…</span>}

      {!loading && tab === 'plugins' && (
        <div style={{ overflowY: 'auto', flex: 1 }}>
          {plugins.length === 0
            ? <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>No plugins registered.</span>
            : plugins.map(p => (
                <div key={p.name} style={{ padding: '8px 0', borderBottom: '1px solid var(--border)' }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <span style={{ fontWeight: 600, fontSize: 13 }}>{p.name}</span>
                    <span className="badge badge-gray">{p.category}</span>
                  </div>
                  {p.description && <div style={{ fontSize: 11, color: 'var(--text-muted)', marginTop: 2 }}>{p.description}</div>}
                  {p.version && <div style={{ fontSize: 11, color: 'var(--text-muted)' }}>v{p.version}</div>}
                </div>
              ))
          }
        </div>
      )}

      {!loading && tab === 'health' && (
        <div style={{ overflowY: 'auto', flex: 1 }}>
          {health.length === 0
            ? <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>No health data.</span>
            : health.map((h, i) => (
                <div key={i} style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '6px 0', borderBottom: '1px solid var(--border)', fontSize: 12 }}>
                  <span style={{ color: 'var(--text-muted)' }}>{h.category} / <span style={{ color: 'var(--text)' }}>{h.name}</span></span>
                  {statusBadge(h.status)}
                </div>
              ))
          }
        </div>
      )}
    </div>
  )
}
