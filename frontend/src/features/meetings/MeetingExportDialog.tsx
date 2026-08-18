import { AlignLeft, Download, FileText, Sparkles, X } from "lucide-react";
import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import type { ExportContent, ExportFormat, ExportMeetingRequest } from "../../contracts/desktop";

interface MeetingExportDialogProps {
  open: boolean;
  title: string;
  busy: boolean;
  onClose: () => void;
  onExport: (request: ExportMeetingRequest) => Promise<boolean>;
}

const CONTENT_OPTIONS: Array<{
  id: ExportContent;
  label: string;
  description: string;
  icon: typeof AlignLeft;
}> = [
  { id: "summary", label: "摘要", description: "标题、内容类型与核心摘要", icon: AlignLeft },
  { id: "transcript", label: "逐字稿", description: "时间戳、说话人与完整正文", icon: FileText },
  { id: "minutes", label: "AI 纪要", description: "主题、结论、待办与风险", icon: Sparkles },
];

/** 渲染可组合内容和格式的会议文档导出窗口。 */
export function MeetingExportDialog({ open, title, busy, onClose, onExport }: MeetingExportDialogProps) {
  const [format, setFormat] = useState<ExportFormat>("docx");
  const [contents, setContents] = useState<ExportContent[]>(CONTENT_OPTIONS.map((option) => option.id));

  useEffect(() => {
    if (!open) return;
    setFormat("docx");
    setContents(CONTENT_OPTIONS.map((option) => option.id));
  }, [open]);

  /** 切换单项导出内容，同时保持稳定的展示顺序。 */
  function toggleContent(content: ExportContent) {
    setContents((current) => current.includes(content)
      ? current.filter((item) => item !== content)
      : CONTENT_OPTIONS.map((option) => option.id).filter((item) => item === content || current.includes(item)));
  }

  /** 提交当前导出选项，并在实际保存成功后关闭窗口。 */
  async function handleSubmit() {
    if (busy || contents.length === 0) return;
    const exported = await onExport({ format, contents });
    if (exported) onClose();
  }

  if (!open) return null;
  return createPortal((
    <div className="dialog-layer export-dialog-layer" role="presentation">
      <button className="export-dialog-backdrop" type="button" aria-label="关闭导出窗口" onClick={onClose} disabled={busy} />
      <section className="export-dialog" role="dialog" aria-modal="true" aria-labelledby="export-dialog-title">
        <button className="icon-button export-dialog-close" type="button" aria-label="关闭导出窗口" onClick={onClose} disabled={busy}><X size={17} /></button>
        <header>
          <span className="eyebrow">导出录音文档</span>
          <h2 id="export-dialog-title">导出“{title}”</h2>
          <p>选择需要的内容，合并保存为一个文档。</p>
        </header>

        <fieldset className="export-content-fieldset">
          <legend>导出内容</legend>
          <div className="export-content-options">
            {CONTENT_OPTIONS.map((option) => {
              const Icon = option.icon;
              return (
                <label key={option.id} className="export-content-option">
                  <input type="checkbox" checked={contents.includes(option.id)} onChange={() => toggleContent(option.id)} disabled={busy} />
                  <span className="export-option-icon" aria-hidden="true"><Icon size={18} /></span>
                  <span><strong>{option.label}</strong><small>{option.description}</small></span>
                </label>
              );
            })}
          </div>
        </fieldset>

        <fieldset className="export-format-fieldset">
          <legend>文件格式</legend>
          <div className="export-format-control" role="radiogroup" aria-label="文件格式">
            <button type="button" role="radio" aria-checked={format === "docx"} onClick={() => setFormat("docx")} disabled={busy}><FileText size={16} />Word <span>.docx</span></button>
            <button type="button" role="radio" aria-checked={format === "pdf"} onClick={() => setFormat("pdf")} disabled={busy}><FileText size={16} />PDF <span>.pdf</span></button>
          </div>
        </fieldset>

        <footer>
          <span className={contents.length === 0 ? "export-selection-warning" : "export-selection-count"}>{contents.length === 0 ? "请至少选择一项内容" : `已选择 ${contents.length} 项内容`}</span>
          <div>
            <button className="button secondary" type="button" onClick={onClose} disabled={busy}>取消</button>
            <button className="button primary" type="button" onClick={() => void handleSubmit()} disabled={busy || contents.length === 0}><Download size={16} />{busy ? "正在生成" : `导出 ${format === "docx" ? "Word" : "PDF"}`}</button>
          </div>
        </footer>
      </section>
    </div>
  ), document.body);
}
