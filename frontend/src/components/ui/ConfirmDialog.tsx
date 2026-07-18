import { AlertTriangle, X } from "lucide-react";
import { useEffect, useRef } from "react";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  busy?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}

/** 渲染带焦点管理的危险操作确认对话框。 */
export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  busy = false,
  onCancel,
  onConfirm,
}: ConfirmDialogProps) {
  const cancelButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) {
      cancelButtonRef.current?.focus();
    }
  }, [open]);

  if (!open) {
    return null;
  }

  return (
    <div className="dialog-layer" role="presentation">
      <div className="dialog" role="alertdialog" aria-modal="true" aria-labelledby="confirm-title" aria-describedby="confirm-description">
        <button className="icon-button dialog-close" type="button" aria-label="关闭确认对话框" onClick={onCancel} disabled={busy}>
          <X size={18} aria-hidden="true" />
        </button>
        <div className="dialog-icon" aria-hidden="true">
          <AlertTriangle size={20} />
        </div>
        <h2 id="confirm-title">{title}</h2>
        <p id="confirm-description">{description}</p>
        <div className="dialog-actions">
          <button ref={cancelButtonRef} className="button secondary" type="button" onClick={onCancel} disabled={busy}>
            返回
          </button>
          <button className="button danger" type="button" onClick={onConfirm} disabled={busy}>
            {busy ? "正在处理" : confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
