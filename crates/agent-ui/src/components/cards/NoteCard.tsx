interface Props {
  content: string
  onChange: (text: string) => void
}

export function NoteCard({ content, onChange }: Props) {
  return (
    <textarea
      className="note-textarea"
      value={content}
      onChange={e => onChange(e.target.value)}
      placeholder="Start typing your note..."
      spellCheck={false}
    />
  )
}
