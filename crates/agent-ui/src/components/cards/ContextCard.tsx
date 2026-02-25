import { useState, useEffect } from 'react'
import { ApiContext } from '../../types'
import { getContext } from '../../services/api'

export function ContextCard() {
  const [ctx, setCtx] = useState<ApiContext | null>(null)
  const [dir, setDir] = useState('')
  const [loading, setLoading] = useState(true)

  const load = (d?: string) => {
    setLoading(true)
    getContext(d || undefined).then(setCtx).catch(() => setCtx(null)).finally(() => setLoading(false))
  }

  useEffect(() => { load() }, [])

  const Row = ({ label, value }: { label: string; value?: string | boolean | null }) => (
    value != null ? (
      <tr>
        <td>{label}</td>
        <td style={{ fontFamily: 'monospace', wordBreak: 'break-all' }}>
          {typeof value === 'boolean'
            ? <span className={`badge ${value ? 'badge-red' : 'badge-green'}`}>{value ? 'dirty' : 'clean'}</span>
            : String(value)}
        </td>
      </tr>
    ) : null
  )

  return (
    <div className="card-inner" style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      <div style={{ display: 'flex', gap: 6 }}>
        <input
          className="search-input"
          style={{ flex: 1, marginBottom: 0 }}
          placeholder="Directory path…"
          value={dir}
          onChange={e => setDir(e.target.value)}
          onKeyDown={e => e.key === 'Enter' && load(dir)}
        />
        <button
          onClick={() => load(dir)}
          style={{ background: 'var(--surface2)', border: '1px solid var(--border)', color: 'var(--text)', padding: '5px 12px', borderRadius: 7, cursor: 'pointer', fontSize: 12 }}
        >↺</button>
      </div>

      {loading && <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Detecting…</span>}

      {!loading && ctx && (
        <>
          {ctx.project && (
            <>
              <div className="section-title">Project</div>
              <table className="info-table">
                <tbody>
                  <Row label="Name" value={ctx.project.name} />
                  <Row label="Type" value={ctx.project.project_type} />
                  <Row label="Path" value={ctx.project.path} />
                  <Row label="Branch" value={ctx.project.git_branch} />
                  <Row label="Remote" value={ctx.project.git_remote} />
                </tbody>
              </table>
            </>
          )}
          {ctx.git && (
            <>
              <div className="section-title">Git</div>
              <table className="info-table">
                <tbody>
                  <Row label="Branch" value={ctx.git.branch} />
                  <Row label="HEAD" value={ctx.git.head_short} />
                  <Row label="Status" value={ctx.git.is_dirty} />
                  <Row label="Root" value={ctx.git.repo_root} />
                </tbody>
              </table>
            </>
          )}
          {ctx.environments.length > 0 && (
            <>
              <div className="section-title">Environments</div>
              <table className="info-table">
                <tbody>
                  {ctx.environments.map(e => (
                    <tr key={e.path}>
                      <td>{e.env_type}</td>
                      <td style={{ fontFamily: 'monospace' }}>{e.name}{e.version ? ` ${e.version}` : ''}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          )}
          {!ctx.project && !ctx.git && ctx.environments.length === 0 && (
            <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>No project context detected.</span>
          )}
        </>
      )}
    </div>
  )
}
