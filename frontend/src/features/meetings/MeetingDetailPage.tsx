import { ArrowLeft, Check, Clipboard, Download, Eye, FileText, ListTodo } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import type { MeetingDetail, SupportedStatement } from "../../contracts/desktop";
import { useDesktopClient } from "../../services/DesktopClientContext";
import { getSafeErrorMessage } from "../../services/desktopClient";
import { useAppStore } from "../../stores/appStore";
import { formatDateTime, formatDuration, formatTimestamp } from "../../utils/format";

type DetailTab = "minutes" | "markdown" | "transcript";

/** 将条目数组转换为适合用户复制的纯文本。 */
function statementsToText(items: SupportedStatement[]): string {
  return items.map((item, index) => `${index + 1}. ${item.content}`).join("\n");
}

/** 渲染结构化纪要及完整逐字稿详情。 */
export function MeetingDetailPage() {
  const client = useDesktopClient();
  const meetingId = useAppStore((state) => state.selectedMeetingId);
  const navigate = useAppStore((state) => state.navigate);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [tab, setTab] = useState<DetailTab>("minutes");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [markdownPreview, setMarkdownPreview] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);

  useEffect(() => {
    if (!meetingId) {
      setError("未选择会议记录");
      setLoading(false);
      return;
    }
    setLoading(true);
    setMarkdownPreview(null);
    setTab("minutes");
    client.getMeetingDetail(meetingId)
      .then((result) => { setDetail(result); setError(null); })
      .catch((reason: unknown) => setError(getSafeErrorMessage(reason)))
      .finally(() => setLoading(false));
  }, [client, meetingId]);

  useEffect(() => {
    if (tab !== "markdown" || !meetingId || markdownPreview !== null) return;
    setPreviewLoading(true);
    client.getMeetingMarkdownPreview(meetingId)
      .then((result) => {
        setMarkdownPreview(result);
        setError(null);
        setPreviewLoading(false);
      })
      .catch((reason: unknown) => {
        setError(getSafeErrorMessage(reason));
        setPreviewLoading(false);
      });
  }, [client, meetingId, tab]);

  const meetingMeta = useMemo(() => {
    if (!detail) return "";
    const { minutes } = detail;
    const time = minutes.meetingTime.startAt ? formatDateTime(minutes.meetingTime.startAt) : "会议时间未提供";
    const participants = minutes.participants.length > 0 ? minutes.participants.join("、") : "参会人未识别";
    return `${time} · ${formatDuration(detail.durationMs)} · ${participants}`;
  }, [detail]);

  /** 复制用户当前请求的已保存文本，并提供通用反馈。 */
  async function handleCopy(value: string | null, label: string) {
    if (!value) return;
    try {
      await navigator.clipboard.writeText(value);
      setNotice(`已复制${label}`);
    } catch {
      setError("复制失败，请检查系统剪贴板权限");
    }
  }

  /** 通过桌面后端导出 UTF-8 Markdown。 */
  async function handleExport() {
    if (!meetingId) return;
    try {
      const result = await client.exportMeetingMarkdown(meetingId);
      if (result.status === "exported") setNotice("Markdown 已导出");
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    }
  }

  if (loading) {
    return <div className="page"><div className="loading-state">正在读取会议详情…</div></div>;
  }

  if (error && !detail) {
    return (
      <div className="page">
        <button className="back-button" type="button" onClick={() => navigate("meetings")}><ArrowLeft size={16} aria-hidden="true" />返回会议记录</button>
        <div className="empty-state error-state" role="alert"><h1>无法打开会议详情</h1><p>{error}</p></div>
      </div>
    );
  }

  if (!detail) return null;
  const { minutes, transcript } = detail;

  return (
    <div className="page detail-page">
      <button className="back-button" type="button" onClick={() => navigate("meetings")}><ArrowLeft size={16} aria-hidden="true" />返回会议记录</button>
      <header className="detail-header">
        <div>
          <span className="eyebrow">{detail.templateName}</span>
          <h1 tabIndex={-1}>{minutes.title ?? "未命名会议"}</h1>
          <p>{meetingMeta}</p>
        </div>
        <div className="header-actions">
          <button className="button secondary" type="button" disabled={!minutes.summary} onClick={() => void handleCopy(minutes.summary, "摘要")}><Clipboard size={16} aria-hidden="true" />复制摘要</button>
          <button className="button primary" type="button" onClick={() => void handleExport()}><Download size={16} aria-hidden="true" />导出 Markdown</button>
        </div>
      </header>

      {notice ? <div className="toast" role="status">{notice}<button type="button" aria-label="关闭提示" onClick={() => setNotice(null)}>×</button></div> : null}
      {error ? <div className="inline-alert error" role="alert"><span>{error}</span><button type="button" onClick={() => setError(null)}>关闭</button></div> : null}

      <div className="detail-tabs" role="tablist" aria-label="会议详情内容">
        <button role="tab" type="button" aria-selected={tab === "minutes"} onClick={() => setTab("minutes")}><ListTodo size={16} aria-hidden="true" />会议纪要</button>
        <button role="tab" type="button" aria-selected={tab === "markdown"} onClick={() => setTab("markdown")}><Eye size={16} aria-hidden="true" />Markdown 预览</button>
        <button role="tab" type="button" aria-selected={tab === "transcript"} onClick={() => setTab("transcript")}><FileText size={16} aria-hidden="true" />完整逐字稿</button>
      </div>

      {tab === "minutes" ? (
        <div className="minutes-layout" role="tabpanel">
          <section className="minutes-section summary-section">
            <SectionHeading title="会议摘要" onCopy={minutes.summary ? () => void handleCopy(minutes.summary, "会议摘要") : undefined} />
            <p className={minutes.summary ? "lead-summary" : "empty-copy"}>{minutes.summary ?? "未提取到相关内容"}</p>
          </section>

          <section className="minutes-section">
            <SectionHeading title="主要议题" />
            {minutes.topics.length > 0 ? <div className="topic-list">{minutes.topics.map((topic, index) => <article key={`${topic.title}-${index}`}><span>{String(index + 1).padStart(2, "0")}</span><div><h3>{topic.title}</h3><p>{topic.summary ?? "未提取到议题摘要"}</p></div></article>)}</div> : <p className="empty-copy">未提取到相关内容</p>}
          </section>

          <StatementSection title="关键结论" items={minutes.conclusions} onCopy={() => void handleCopy(statementsToText(minutes.conclusions), "关键结论")} />
          <StatementSection title="决策事项" items={minutes.decisions} onCopy={() => void handleCopy(statementsToText(minutes.decisions), "决策事项")} />

          <section className="minutes-section">
            <SectionHeading title="待办事项" />
            {minutes.actionItems.length > 0 ? (
              <div className="table-wrap"><table><thead><tr><th>事项</th><th>负责人</th><th>截止日期</th></tr></thead><tbody>{minutes.actionItems.map((item, index) => <tr key={`${item.description}-${index}`}><td>{item.description}</td><td>{item.owner ?? "未指定"}</td><td>{item.dueDate ?? item.dueDateText ?? "未指定"}</td></tr>)}</tbody></table></div>
            ) : <p className="empty-copy">未提取到相关内容</p>}
          </section>

          <section className="minutes-section">
            <SectionHeading title="风险和问题" />
            {minutes.risksAndIssues.length > 0 ? <div className="risk-list">{minutes.risksAndIssues.map((item, index) => <article key={`${item.description}-${index}`}><span className={`risk-kind ${item.kind}`}>{item.kind === "risk" ? "风险" : "问题"}</span><div><h3>{item.description}</h3>{item.impact ? <p><strong>影响：</strong>{item.impact}</p> : null}{item.mitigation ? <p><strong>应对：</strong>{item.mitigation}</p> : null}</div></article>)}</div> : <p className="empty-copy">未提取到相关内容</p>}
          </section>
        </div>
      ) : tab === "transcript" ? (
        <section className="transcript-panel" role="tabpanel">
          <div className="section-heading transcript-heading"><div><h2>完整逐字稿</h2><p>{transcript.language ?? "语言未标注"} · {transcript.segments.length > 0 ? `${transcript.segments.length} 个分段` : "纯文本"}</p></div><button className="button secondary" type="button" onClick={() => void handleCopy(transcript.text, "完整逐字稿")}><Clipboard size={16} aria-hidden="true" />复制全文</button></div>
          {transcript.segments.length > 0 ? <div className="transcript-segments">{transcript.segments.map((segment) => <article key={segment.id}><div className="segment-meta">{segment.startMs !== undefined ? <time>{formatTimestamp(segment.startMs)}</time> : null}{segment.speakerLabel ? <span>{segment.speakerLabel}</span> : null}{segment.confidence !== undefined && segment.confidence < 0.7 ? <span className="low-confidence">需核对</span> : null}</div><p>{segment.text}</p></article>)}</div> : <pre className="transcript-plain">{transcript.text}</pre>}
        </section>
      ) : (
        <section className="markdown-preview-panel" role="tabpanel" aria-label="Markdown 文档预览">
          <div className="preview-toolbar">
            <div><span className="eyebrow">本地预览</span><h2>导出文档效果</h2><p>预览内容与导出的 Markdown 文本一致；文档中的原始 HTML 不会执行。</p></div>
            <button className="button secondary" type="button" onClick={() => void handleExport()}><Download size={16} aria-hidden="true" />导出 Markdown</button>
          </div>
          {previewLoading ? <div className="preview-loading">正在生成预览…</div> : null}
          {!previewLoading && markdownPreview !== null ? (
            <article className="markdown-paper">
              <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml>{markdownPreview}</ReactMarkdown>
            </article>
          ) : null}
        </section>
      )}
    </div>
  );
}

interface SectionHeadingProps {
  title: string;
  onCopy?: () => void;
}

/** 渲染纪要区块标题及可选复制操作。 */
function SectionHeading({ title, onCopy }: SectionHeadingProps) {
  return <div className="minutes-heading"><h2>{title}</h2>{onCopy ? <button className="section-copy" type="button" onClick={onCopy}><Clipboard size={14} aria-hidden="true" />复制本节</button> : null}</div>;
}

interface StatementSectionProps {
  title: string;
  items: SupportedStatement[];
  onCopy: () => void;
}

/** 渲染结论或决策等有序事实列表。 */
function StatementSection({ title, items, onCopy }: StatementSectionProps) {
  return (
    <section className="minutes-section">
      <SectionHeading title={title} onCopy={items.length > 0 ? onCopy : undefined} />
      {items.length > 0 ? <ul className="statement-list">{items.map((item, index) => <li key={`${item.content}-${index}`}><Check size={16} aria-hidden="true" /><span>{item.content}</span></li>)}</ul> : <p className="empty-copy">未提取到相关内容</p>}
    </section>
  );
}
