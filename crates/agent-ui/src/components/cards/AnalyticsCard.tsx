import { useState, useEffect } from 'react'
import ReactMarkdown from 'react-markdown'
import { ApiAnalyticsSummary } from '../../types'
import { getAnalyticsSummary, getAnalyticsReport } from '../../services/api'

export function AnalyticsCard() {
  const [summary, setSummary] = useState<ApiAnalyticsSummary | null>(null)
  const [report, setReport] = useState<string | null>(null)
  const [period, setPeriod] = useState<'week' | 'month'>('week')
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    setLoading(true)
    Promise.all([
      getAnalyticsSummary(),
      getAnalyticsReport('week'),
    ]).then(([s, r]) => {
      setSummary(s)
      setReport(r)
    }).catch(() => {}).finally(() => setLoading(false))
  }, [])

  const loadReport = (p: 'week' | 'month') => {
    setPeriod(p)
    setReport(null)
    getAnalyticsReport(p).then(setReport).catch(() => {})
  }

  if (loading) return <div className="card-inner" style={{ color: 'var(--text-muted)' }}>Loading analytics…</div>
  if (!summary) return <div className="card-inner" style={{ color: 'var(--text-muted)' }}>No analytics data.</div>

  const avg = summary.average_session_duration_secs
    ? (() => { const m = Math.floor(summary.average_session_duration_secs! / 60); const h = Math.floor(m / 60); return h > 0 ? `${h}h ${m % 60}m` : `${m}m` })()
    : '—'

  const maxCount = summary.top_tools[0]?.[1] ?? 1

  return (
    <div className="card-inner">
      <div className="stats-grid">
        {([
          [summary.total_sessions, 'Sessions'],
          [summary.active_days, 'Active Days'],
          [avg, 'Avg Session'],
          [summary.deep_work_sessions, 'Deep Work'],
        ] as [number | string, string][]).map(([v, l]) => (
          <div key={l} className="stat-card">
            <div className="stat-value">{v}</div>
            <div className="stat-label">{l}</div>
          </div>
        ))}
      </div>

      {summary.today && (
        <>
          <div className="section-title">Today</div>
          <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginBottom: 10 }}>
            {[
              `${summary.today.sessions} sessions`,
              `${summary.today.messages} msgs`,
              summary.today.active_time,
              `${summary.today.tool_calls} tools`,
              ...(summary.today.tool_errors > 0 ? [`${summary.today.tool_errors} errors`] : []),
            ].map(t => <span key={t} className={`badge ${t.includes('errors') ? 'badge-red' : 'badge-gray'}`}>{t}</span>)}
          </div>
        </>
      )}

      {summary.top_tools.length > 0 && (
        <>
          <div className="section-title">Top Tools</div>
          <div style={{ marginBottom: 10 }}>
            {summary.top_tools.slice(0, 8).map(([name, count]) => (
              <div key={name} className="bar-row">
                <span className="bar-label">{name}</span>
                <div className="bar-track"><div className="bar-fill" style={{ width: `${(count / maxCount) * 100}%` }} /></div>
                <span className="bar-count">{count}</span>
              </div>
            ))}
          </div>
        </>
      )}

      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <div className="section-title" style={{ margin: 0 }}>Report</div>
        <div className="tab-row">
          <button className={`tab-btn${period === 'week' ? ' active' : ''}`} onClick={() => loadReport('week')}>Week</button>
          <button className={`tab-btn${period === 'month' ? ' active' : ''}`} onClick={() => loadReport('month')}>Month</button>
        </div>
      </div>
      <div style={{ marginTop: 8, background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 7, padding: '10px 12px', fontSize: 12 }}>
        {report
          ? <div className="md"><ReactMarkdown>{report}</ReactMarkdown></div>
          : <span style={{ color: 'var(--text-muted)' }}>Loading…</span>}
      </div>
    </div>
  )
}
