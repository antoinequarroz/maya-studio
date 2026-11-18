/** Fenêtres modales : base, saisie d'un texte et confirmation. */

import { type FormEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { IconClose } from "./Icons";

export function Modal({
  title,
  onClose,
  children,
  footer,
  width = 420,
}: {
  title: string;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [onClose]);

  return (
    <div className="modal-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal" role="dialog" aria-modal="true" aria-label={title} style={{ width }}>
        <header className="modal-header">
          <h2>{title}</h2>
          <button type="button" className="icon-button" onClick={onClose} title="Fermer">
            <IconClose />
          </button>
        </header>
        <div className="modal-body">{children}</div>
        {footer && <footer className="modal-footer">{footer}</footer>}
      </div>
    </div>
  );
}

export function PromptDialog({
  title,
  label,
  initial = "",
  confirmLabel = "Valider",
  onSubmit,
  onClose,
}: {
  title: string;
  label: string;
  initial?: string;
  confirmLabel?: string;
  onSubmit: (value: string) => void;
  onClose: () => void;
}) {
  const [value, setValue] = useState(initial);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    input.current?.focus();
    input.current?.select();
  }, []);
  const submit = (e: FormEvent) => {
    e.preventDefault();
    const v = value.trim();
    if (v) onSubmit(v);
  };
  return (
    <Modal title={title} onClose={onClose}>
      <form onSubmit={submit} className="form">
        <label className="field">
          <span>{label}</span>
          <input ref={input} value={value} onChange={(e) => setValue(e.target.value)} />
        </label>
        <div className="modal-actions">
          <button type="button" className="button" onClick={onClose}>
            Annuler
          </button>
          <button type="submit" className="button primary" disabled={!value.trim()}>
            {confirmLabel}
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function ConfirmDialog({
  title,
  message,
  confirmLabel,
  onConfirm,
  onClose,
}: {
  title: string;
  message: ReactNode;
  confirmLabel: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const ok = useRef<HTMLButtonElement>(null);
  useEffect(() => ok.current?.focus(), []);
  return (
    <Modal title={title} onClose={onClose}>
      <p className="modal-text">{message}</p>
      <div className="modal-actions">
        <button type="button" className="button" onClick={onClose}>
          Annuler
        </button>
        <button ref={ok} type="button" className="button danger" onClick={onConfirm}>
          {confirmLabel}
        </button>
      </div>
    </Modal>
  );
}
