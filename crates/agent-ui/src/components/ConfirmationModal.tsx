interface Props {
  title: string
  message: string
  onConfirm: () => void
  onCancel: () => void
}

export function ConfirmationModal({ title, message, onConfirm, onCancel }: Props) {
  return (
    <div className="modal-overlay" onClick={onCancel}>
      <div className="modal-box" onClick={e => e.stopPropagation()} style={{ width: 380 }}>
        <div className="modal-hd">
          <h2>{title}</h2>
          <button className="modal-close" onClick={onCancel}>×</button>
        </div>
        <div className="modal-bd">
          <p style={{ color: 'var(--text-muted)', fontSize: 13 }}>{message}</p>
          <div className="confirm-btns">
            <button className="btn-secondary" onClick={onCancel}>Cancel</button>
            <button className="btn-danger" onClick={onConfirm}>Delete</button>
          </div>
        </div>
      </div>
    </div>
  )
}
