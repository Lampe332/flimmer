import { useEffect, useRef } from 'react'
import { useI18n } from '../i18n'
import Icon from './Icon'

type Props = { youtubeKey: string; title: string; onClose: () => void }

export default function TrailerModal({ youtubeKey, title, onClose }: Props) {
  const ref = useRef<HTMLDialogElement>(null)
  const { t } = useI18n()

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
      aria-label={t('trailer.label', { title })}
      onClose={onClose}
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose()
      }}
    >
      <div className="modal-box">
        <div className="modal-head">
          <h2>{t('trailer.heading', { title })}</h2>
          <button type="button" className="icon-btn" onClick={onClose} aria-label={t('trailer.close')} autoFocus>
            <Icon name="close" />
          </button>
        </div>
        <div className="frame">
          <iframe
            src={`https://www.youtube-nocookie.com/embed/${encodeURIComponent(youtubeKey)}?autoplay=1&rel=0`}
            title={t('trailer.label', { title })}
            allow="autoplay; fullscreen; encrypted-media; picture-in-picture"
            allowFullScreen
            referrerPolicy="strict-origin-when-cross-origin"
          />
        </div>
      </div>
    </dialog>
  )
}
