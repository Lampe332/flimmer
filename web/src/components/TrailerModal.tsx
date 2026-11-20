import { useEffect, useRef } from 'react'
import Icon from './Icon'

type Props = { youtubeKey: string; title: string; onClose: () => void }

export default function TrailerModal({ youtubeKey, title, onClose }: Props) {
  const ref = useRef<HTMLDialogElement>(null)

  useEffect(() => {
    const d = ref.current
    if (!d) return
    d.showModal()
    return () => d.close()
  }, [])

  return (
    <dialog
      ref={ref}
      className="modal"
      aria-label={`Trailer: ${title}`}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className="modal-box">
        <div className="modal-head">
          <h2>Trailer zu „{title}“</h2>
          <button type="button" className="icon-btn" onClick={onClose} aria-label="Trailer schließen" autoFocus>
            <Icon name="close" />
          </button>
        </div>
        <div className="frame">
          <iframe
            src={`https://www.youtube-nocookie.com/embed/${encodeURIComponent(youtubeKey)}?autoplay=1&rel=0`}
            title={`Trailer: ${title}`}
            allow="autoplay; fullscreen; encrypted-media; picture-in-picture"
            allowFullScreen
            referrerPolicy="strict-origin-when-cross-origin"
          />
        </div>
      </div>
    </dialog>
  )
}
