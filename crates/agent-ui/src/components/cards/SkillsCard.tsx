import { useState, useEffect, useCallback } from 'react'
import ReactMarkdown from 'react-markdown'
import { ApiSkill } from '../../types'
import { listSkills, searchSkills, getSkillContent } from '../../services/api'

export function SkillsCard() {
  const [query, setQuery] = useState('')
  const [skills, setSkills] = useState<ApiSkill[]>([])
  const [selected, setSelected] = useState<ApiSkill | null>(null)
  const [content, setContent] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    listSkills().then(setSkills).catch(() => {}).finally(() => setLoading(false))
  }, [])

  const doSearch = useCallback((q: string) => {
    if (!q.trim()) {
      listSkills().then(setSkills).catch(() => {})
    } else {
      searchSkills(q).then(setSkills).catch(() => {})
    }
  }, [])

  useEffect(() => {
    const t = setTimeout(() => doSearch(query), 280)
    return () => clearTimeout(t)
  }, [query, doSearch])

  const openSkill = (skill: ApiSkill) => {
    setSelected(skill)
    setContent(null)
    getSkillContent(skill.name).then(setContent).catch(() => setContent('Failed to load skill content.'))
  }

  if (selected) {
    return (
      <div className="card-inner" style={{ display: 'flex', flexDirection: 'column' }}>
        <button
          onClick={() => { setSelected(null); setContent(null) }}
          style={{ background: 'none', border: 'none', color: 'var(--accent)', cursor: 'pointer', fontSize: 12, marginBottom: 8, textAlign: 'left', padding: 0 }}
        >← Back</button>
        <div style={{ fontWeight: 600, marginBottom: 4 }}>{selected.name}</div>
        <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginBottom: 10 }}>
          {selected.tags.map(t => <span key={t} className="badge badge-blue">{t}</span>)}
        </div>
        <div style={{ flex: 1, overflowY: 'auto', background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 7, padding: '10px 12px', fontSize: 12 }}>
          {content
            ? <div className="md"><ReactMarkdown>{content}</ReactMarkdown></div>
            : <span style={{ color: 'var(--text-muted)' }}>Loading…</span>}
        </div>
      </div>
    )
  }

  return (
    <div className="card-inner" style={{ display: 'flex', flexDirection: 'column' }}>
      <input
        className="search-input"
        placeholder="Search skills…"
        value={query}
        onChange={e => setQuery(e.target.value)}
      />
      <div className="skill-list" style={{ flex: 1, overflowY: 'auto' }}>
        {loading && <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>Loading…</span>}
        {!loading && skills.length === 0 && <span style={{ color: 'var(--text-muted)', fontSize: 12 }}>No skills found.</span>}
        {skills.map(skill => (
          <div key={skill.name} className="skill-item" onClick={() => openSkill(skill)}>
            <div className="skill-name">{skill.name}</div>
            <div className="skill-desc">{skill.description}</div>
            {skill.tags.length > 0 && (
              <div className="skill-tags">
                {skill.tags.slice(0, 4).map(t => <span key={t} className="badge badge-gray">{t}</span>)}
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  )
}
