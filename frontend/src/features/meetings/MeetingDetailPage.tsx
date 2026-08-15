import {
  AlertTriangle,
  ArrowLeft,
  Check,
  Clipboard,
  CirclePlay,
  Download,
  Eye,
  FileAudio,
  FileText,
  ListTree,
  Search,
  Sparkles,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { ConfirmDialog } from "../../components/ui/ConfirmDialog";
import type {
  ContentType,
  MeetingDetail,
  MeetingMinutes,
  SupportedStatement,
  Topic,
  TranscriptSegment,
} from "../../contracts/desktop";
import { useDesktopClient } from "../../services/DesktopClientContext";
import { getSafeErrorMessage } from "../../services/desktopClient";
import { useAppStore } from "../../stores/appStore";
import { formatDateTime, formatDuration, formatTimestamp } from "../../utils/format";

type DetailTab = "transcript" | "minutes" | "chapters";
const TRANSCRIPT_PARAGRAPH_TARGET = 180;

interface ContentPresentation {
  label: string;
  summaryHeading: string;
  topicsHeading: string;
  insightsHeading: string;
}

interface ChapterEntry {
  topic: Topic;
  index: number;
  startMs?: number;
  segmentIndex?: number;
}

const CONTENT_PRESENTATIONS: Record<ContentType, ContentPresentation> = {
  meeting: { label: "会议", summaryHeading: "会议摘要", topicsHeading: "主要议题", insightsHeading: "关键结论" },
  speech: { label: "演讲", summaryHeading: "内容概览", topicsHeading: "演讲脉络", insightsHeading: "核心观点" },
  lecture: { label: "讲座", summaryHeading: "内容概览", topicsHeading: "讲座脉络", insightsHeading: "核心观点" },
  course: { label: "课程", summaryHeading: "课程概览", topicsHeading: "知识脉络", insightsHeading: "核心知识点" },
  interview: { label: "访谈", summaryHeading: "访谈概览", topicsHeading: "话题脉络", insightsHeading: "关键洞察" },
  report: { label: "汇报", summaryHeading: "汇报概览", topicsHeading: "内容脉络", insightsHeading: "关键结论" },
  article_material: { label: "口述素材", summaryHeading: "内容概览", topicsHeading: "章节结构", insightsHeading: "核心观点" },
  other: { label: "录音内容", summaryHeading: "内容概览", topicsHeading: "主题脉络", insightsHeading: "内容要点" },
};

/** 将条目数组转换为适合用户复制的纯文本。 */
function statementsToText(items: SupportedStatement[]): string {
  return items.map((item, index) => `${index + 1}. ${item.content}`).join("\n");
}

/** 将没有结构化分段的历史逐字稿按原有换行和句末标点整理为可读段落。 */
export function splitTranscriptParagraphs(text: string): string[] {
  const paragraphs: string[] = [];
  const sourceLines = text.split(/\r?\n/).map((line) => line.trim()).filter(Boolean);

  for (const line of sourceLines) {
    const sentences = line.match(/[^。！？!?；;.]+[。！？!?；;.]+|[^。！？!?；;.]+$/g) ?? [line];
    let paragraph = "";
    for (const sentence of sentences) {
      const normalized = sentence.trim();
      if (!normalized) continue;
      if (paragraph && paragraph.length + normalized.length > TRANSCRIPT_PARAGRAPH_TARGET) {
        paragraphs.push(paragraph);
        paragraph = normalized;
      } else {
        paragraph += normalized;
      }
    }
    if (paragraph) paragraphs.push(paragraph);
  }

  return paragraphs.length > 0 ? paragraphs : [text];
}

/** 解析新记录的显式内容类型，并为旧记录按手动模板提供保守兼容。 */
export function resolveContentType(minutes: MeetingMinutes, templateName: string): ContentType {
  if (minutes.contentType && CONTENT_PRESENTATIONS[minutes.contentType]) return minutes.contentType;
  if (/标准会议|项目周会|客户沟通/.test(templateName)) return "meeting";
  if (/演讲/.test(templateName)) return "speech";
  if (/讲座/.test(templateName)) return "lecture";
  if (/课程/.test(templateName)) return "course";
  if (/访谈|专访/.test(templateName)) return "interview";
  if (/研究|计划书/.test(templateName)) return "report";
  if (/文章|大纲/.test(templateName)) return "article_material";
  return "other";
}

/** 根据议题证据定位最早转写时间，生成可跳转的章节列表。 */
export function buildChapterEntries(topics: Topic[], segments: TranscriptSegment[]): ChapterEntry[] {
  const segmentLookup = new Map(segments.map((segment, index) => [segment.id, { segment, index }]));
  return topics.map((topic, index) => {
    const candidates = topic.evidenceSegmentIds
      .map((id) => segmentLookup.get(id))
      .filter((entry): entry is { segment: TranscriptSegment; index: number } => entry !== undefined)
      .filter((entry) => entry.segment.startMs !== undefined)
      .sort((left, right) => (left.segment.startMs ?? 0) - (right.segment.startMs ?? 0));
    return {
      topic,
      index,
      startMs: candidates[0]?.segment.startMs,
      segmentIndex: candidates[0]?.index,
    };
  });
}

/** 渲染结构化录音总结、章节和完整逐字稿详情。 */
export function MeetingDetailPage() {
  const client = useDesktopClient();
  const recordId = useAppStore((state) => state.selectedMeetingId);
  const navigate = useAppStore((state) => state.navigate);
  const [detail, setDetail] = useState<MeetingDetail | null>(null);
  const [tab, setTab] = useState<DetailTab>("minutes");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [transcriptQuery, setTranscriptQuery] = useState("");
  const [highlightedSegmentIndex, setHighlightedSegmentIndex] = useState<number | null>(null);
  const [markdownPreview, setMarkdownPreview] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [deleteOpen, setDeleteOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [openingPlayback, setOpeningPlayback] = useState(false);

  useEffect(() => {
    if (!recordId) {
      setError("未选择录音记录");
      setLoading(false);
      return;
    }
    setLoading(true);
    setMarkdownPreview(null);
    setPreviewOpen(false);
    setTab("minutes");
    setTranscriptQuery("");
    client.getMeetingDetail(recordId)
      .then((result) => { setDetail(result); setError(null); })
      .catch((reason: unknown) => setError(getSafeErrorMessage(reason)))
      .finally(() => setLoading(false));
  }, [client, recordId]);

  useEffect(() => {
    if (!previewOpen || !recordId || markdownPreview !== null) return;
    setPreviewLoading(true);
    client.getMeetingMarkdownPreview(recordId)
      .then((result) => {
        setMarkdownPreview(result);
        setError(null);
      })
      .catch((reason: unknown) => setError(getSafeErrorMessage(reason)))
      .finally(() => setPreviewLoading(false));
  }, [client, markdownPreview, previewOpen, recordId]);

  const contentType = detail ? resolveContentType(detail.minutes, detail.templateName) : "other";
  const presentation = CONTENT_PRESENTATIONS[contentType];
  const chapters = useMemo(
    () => detail ? buildChapterEntries(detail.minutes.topics, detail.transcript.segments) : [],
    [detail],
  );
  const normalizedQuery = transcriptQuery.trim().toLocaleLowerCase("zh-CN");
  const visibleSegments = useMemo(() => {
    if (!detail) return [];
    return detail.transcript.segments
      .map((segment, index) => ({ segment, index }))
      .filter(({ segment }) => !normalizedQuery
        || `${segment.speakerLabel ?? ""} ${segment.text}`.toLocaleLowerCase("zh-CN").includes(normalizedQuery));
  }, [detail, normalizedQuery]);
  const visibleParagraphs = useMemo(() => {
    if (!detail || detail.transcript.segments.length > 0) return [];
    return splitTranscriptParagraphs(detail.transcript.text)
      .filter((paragraph) => !normalizedQuery || paragraph.toLocaleLowerCase("zh-CN").includes(normalizedQuery));
  }, [detail, normalizedQuery]);

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
    if (!recordId) return;
    try {
      const result = await client.exportMeetingMarkdown(recordId);
      if (result.status === "exported") setNotice("Markdown 已导出");
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    }
  }

  /** 使用系统播放器试听当前记录关联的原始媒体。 */
  async function handlePlayback() {
    if (!recordId || openingPlayback) return;
    setOpeningPlayback(true);
    setError(null);
    try {
      const result = await client.playMeetingMedia(recordId);
      if (result.status === "opened") {
        setNotice(result.reboundSource ? "已关联原文件，并使用系统播放器打开" : "已使用系统播放器打开");
      }
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setOpeningPlayback(false);
    }
  }

  /** 删除当前本地记录及关联任务，并返回记录列表。 */
  async function handleDelete() {
    if (!recordId) return;
    setDeleting(true);
    setError(null);
    try {
      const deleted = await client.deleteMeeting(recordId);
      if (!deleted) {
        setError("录音记录不存在或已被删除");
        setDeleteOpen(false);
        return;
      }
      navigate("meetings");
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setDeleting(false);
    }
  }

  /** 从章节切换到逐字稿，并定位到对应的最早证据分段。 */
  function handleChapterJump(chapter: ChapterEntry) {
    if (chapter.segmentIndex === undefined) return;
    setHighlightedSegmentIndex(chapter.segmentIndex);
    setTranscriptQuery("");
    setTab("transcript");
    window.setTimeout(() => {
      const target = document.getElementById(`transcript-segment-${chapter.segmentIndex}`);
      if (target && typeof target.scrollIntoView === "function") {
        target.scrollIntoView({ behavior: "smooth", block: "center" });
      }
    }, 0);
  }

  if (loading) {
    return <div className="page"><div className="loading-state">正在读取录音详情…</div></div>;
  }

  if (error && !detail) {
    return (
      <div className="page">
        <button className="back-button" type="button" onClick={() => navigate("meetings")}><ArrowLeft size={16} aria-hidden="true" />返回录音记录</button>
        <div className="empty-state error-state" role="alert"><h1>无法打开录音详情</h1><p>{error}</p></div>
      </div>
    );
  }

  if (!detail) return null;
  const { minutes, transcript } = detail;
  const metaItems = [
    detail.sourceName,
    `录音 ${formatDuration(detail.durationMs)}`,
    `${transcript.segments.length || chapters.length} 个分段`,
    `处理 ${formatDuration(detail.processingDurationMs)}`,
  ];
  if (contentType === "meeting" && minutes.meetingTime.startAt) {
    metaItems.unshift(formatDateTime(minutes.meetingTime.startAt));
  }

  return (
    <div className={`page detail-page record-workbench content-${contentType}`}>
      <div className="record-topbar">
        <button className="back-button" type="button" onClick={() => navigate("meetings")}><ArrowLeft size={16} aria-hidden="true" />返回录音记录</button>
        <div className="header-actions">
          <button className="button quiet" type="button" disabled={openingPlayback} onClick={() => void handlePlayback()}><CirclePlay size={17} aria-hidden="true" />{openingPlayback ? "正在打开" : "试听"}</button>
          <button className="button quiet" type="button" onClick={() => setPreviewOpen(true)}><Eye size={16} aria-hidden="true" />文档预览</button>
          <button className="button secondary" type="button" disabled={!minutes.summary} onClick={() => void handleCopy(minutes.summary, "AI 摘要")}><Clipboard size={16} aria-hidden="true" />复制摘要</button>
          <button className="button primary" type="button" onClick={() => void handleExport()}><Download size={16} aria-hidden="true" />导出</button>
          <button className="icon-button delete-icon-action" type="button" aria-label="删除录音记录" title="删除录音记录" onClick={() => setDeleteOpen(true)}><Trash2 size={17} aria-hidden="true" /></button>
        </div>
      </div>

      <header className="record-heading">
        <div>
          <div className="record-kicker"><span>{presentation.label}</span><span>{detail.templateName}</span></div>
          <h1 tabIndex={-1}>{minutes.title ?? "未命名录音"}</h1>
          <p>{metaItems.join(" · ")}</p>
        </div>
      </header>

      <section className="recording-strip" aria-label="录音结构概览">
        <div className="recording-source"><FileAudio size={19} aria-hidden="true" /><div><strong>{detail.sourceName}</strong><span>{formatDateTime(detail.createdAt)} 创建</span></div></div>
        <div className="recording-track" aria-hidden="true">
          {(chapters.length > 0 ? chapters : Array.from({ length: 6 }, (_, index) => ({ index }))).map((chapter) => <span key={chapter.index} />)}
        </div>
        <div className="recording-duration"><strong>{formatDuration(detail.durationMs)}</strong><span>{transcript.language ?? "语言未标注"}</span></div>
      </section>

      <ConfirmDialog
        open={deleteOpen}
        title="删除录音记录？"
        description="逐字稿、AI 总结和关联任务将从本机永久删除；用户导入的原始文件不会受影响。"
        confirmLabel="删除记录"
        busy={deleting}
        onCancel={() => setDeleteOpen(false)}
        onConfirm={() => void handleDelete()}
      />

      {notice ? <div className="toast" role="status">{notice}<button type="button" aria-label="关闭提示" onClick={() => setNotice(null)}>×</button></div> : null}
      {error ? <div className="inline-alert error" role="alert"><span>{error}</span><button type="button" onClick={() => setError(null)}>关闭</button></div> : null}

      <div className="detail-tabs" role="tablist" aria-label="录音详情内容">
        <button role="tab" type="button" aria-selected={tab === "transcript"} onClick={() => setTab("transcript")}><FileText size={16} aria-hidden="true" />逐字稿</button>
        <button role="tab" type="button" aria-selected={tab === "minutes"} onClick={() => setTab("minutes")}><Sparkles size={16} aria-hidden="true" />AI 纪要</button>
        <button role="tab" type="button" aria-selected={tab === "chapters"} onClick={() => setTab("chapters")}><ListTree size={16} aria-hidden="true" />章节</button>
      </div>

      {tab === "minutes" ? (
        <div className="minutes-layout" role="tabpanel">
          <div className="content-profile"><Sparkles size={15} aria-hidden="true" /><strong>{presentation.label}整理</strong><span>{detail.templateName}</span></div>

          <section className="minutes-section summary-section">
            <SectionHeading title={presentation.summaryHeading} onCopy={minutes.summary ? () => void handleCopy(minutes.summary, presentation.summaryHeading) : undefined} />
            <div className="summary-callout"><span aria-hidden="true">“</span><p className={minutes.summary ? "lead-summary" : "empty-copy"}>{minutes.summary ?? "未提取到相关内容"}</p></div>
          </section>

          <section className="minutes-section">
            <SectionHeading title={presentation.topicsHeading} />
            {minutes.topics.length > 0 ? (
              <div className="topic-grid">{minutes.topics.map((topic, index) => (
                <article key={`${topic.title}-${index}`}>
                  <span>{String(index + 1).padStart(2, "0")}</span>
                  <h3>{topic.title}</h3>
                  <p>{topic.summary ?? "未提取到本节概括"}</p>
                </article>
              ))}</div>
            ) : <p className="empty-copy">未提取到主题脉络</p>}
          </section>

          <StatementSection title={presentation.insightsHeading} items={minutes.conclusions} onCopy={() => void handleCopy(statementsToText(minutes.conclusions), presentation.insightsHeading)} />
          {minutes.decisions.length > 0 ? <StatementSection title="决策事项" items={minutes.decisions} onCopy={() => void handleCopy(statementsToText(minutes.decisions), "决策事项")} /> : null}

          {minutes.actionItems.length > 0 ? (
            <section className="minutes-section">
              <SectionHeading title="待办事项" />
              <div className="table-wrap"><table><thead><tr><th>事项</th><th>负责人</th><th>截止日期</th></tr></thead><tbody>{minutes.actionItems.map((item, index) => <tr key={`${item.description}-${index}`}><td>{item.description}</td><td>{item.owner ?? "未指定"}</td><td>{item.dueDate ?? item.dueDateText ?? "未指定"}</td></tr>)}</tbody></table></div>
            </section>
          ) : null}

          {minutes.risksAndIssues.length > 0 ? (
            <section className="minutes-section">
              <SectionHeading title="风险和问题" />
              <div className="risk-list">{minutes.risksAndIssues.map((item, index) => <article key={`${item.description}-${index}`}><span className={`risk-kind ${item.kind}`}>{item.kind === "risk" ? "风险" : "问题"}</span><div><h3>{item.description}</h3>{item.impact ? <p><strong>影响：</strong>{item.impact}</p> : null}{item.mitigation ? <p><strong>应对：</strong>{item.mitigation}</p> : null}</div></article>)}</div>
            </section>
          ) : null}
        </div>
      ) : tab === "transcript" ? (
        <section className="transcript-panel" role="tabpanel">
          <div className="transcript-toolbar">
            <label className="transcript-search"><Search size={16} aria-hidden="true" /><span className="visually-hidden">查找逐字稿</span><input type="search" value={transcriptQuery} onChange={(event) => setTranscriptQuery(event.target.value)} placeholder="查找逐字稿" />{transcriptQuery ? <button type="button" aria-label="清除查找" onClick={() => setTranscriptQuery("")}><X size={14} /></button> : null}</label>
            <span>{normalizedQuery ? `${visibleSegments.length || visibleParagraphs.length} 个匹配` : `${transcript.segments.length || visibleParagraphs.length} 个分段`}</span>
            <button className="button secondary" type="button" onClick={() => void handleCopy(transcript.text, "完整逐字稿")}><Clipboard size={16} aria-hidden="true" />复制全文</button>
          </div>
          {transcript.segments.length > 0 ? (
            visibleSegments.length > 0 ? <div className="transcript-segments">{visibleSegments.map(({ segment, index }) => <article id={`transcript-segment-${index}`} className={highlightedSegmentIndex === index ? "highlighted" : undefined} key={segment.id}><div className="segment-meta">{segment.speakerLabel ? <strong>{segment.speakerLabel}</strong> : <strong>录音内容</strong>}{segment.startMs !== undefined ? <time>{formatTimestamp(segment.startMs)}</time> : null}{segment.confidence !== undefined && segment.confidence < 0.7 ? <span className="low-confidence">需核对</span> : null}</div><p>{segment.text}</p></article>)}</div>
              : <div className="transcript-empty"><Search size={20} aria-hidden="true" /><p>没有找到匹配的逐字稿内容</p></div>
          ) : visibleParagraphs.length > 0 ? <div className="transcript-plain">{visibleParagraphs.map((paragraph, index) => <p key={`${index}-${paragraph.slice(0, 24)}`}>{paragraph}</p>)}</div>
            : <div className="transcript-empty"><Search size={20} aria-hidden="true" /><p>没有找到匹配的逐字稿内容</p></div>}
        </section>
      ) : (
        <section className="chapters-panel" role="tabpanel">
          <div className="chapters-heading"><div><span className="eyebrow">内容导航</span><h2>{presentation.topicsHeading}</h2></div><p>{chapters.length} 个章节</p></div>
          {chapters.length > 0 ? <ol className="chapter-timeline">{chapters.map((chapter) => <li key={`${chapter.topic.title}-${chapter.index}`}><div className="chapter-time">{chapter.segmentIndex !== undefined ? <button type="button" onClick={() => handleChapterJump(chapter)}>{chapter.startMs !== undefined ? formatTimestamp(chapter.startMs) : `章节 ${chapter.index + 1}`}</button> : <span>章节 {chapter.index + 1}</span>}</div><div className="chapter-marker" aria-hidden="true" /><article><span>{String(chapter.index + 1).padStart(2, "0")}</span><h3>{chapter.topic.title}</h3><p>{chapter.topic.summary ?? "未提取到本节概括"}</p></article></li>)}</ol>
            : <div className="transcript-empty"><ListTree size={22} aria-hidden="true" /><p>当前记录没有可用章节</p></div>}
        </section>
      )}

      {previewOpen ? createPortal((
        <div className="document-preview-layer" role="dialog" aria-modal="true" aria-label="Markdown 文档预览">
          <button className="document-preview-backdrop" type="button" aria-label="关闭文档预览" onClick={() => setPreviewOpen(false)} />
          <section className="document-preview-window">
            <div className="preview-toolbar"><div><span className="eyebrow">导出预览</span><h2>Markdown 文档</h2><p>预览内容与导出的文档一致</p></div><div className="header-actions"><button className="button secondary" type="button" onClick={() => void handleExport()}><Download size={16} aria-hidden="true" />导出</button><button className="icon-button" type="button" aria-label="关闭文档预览" onClick={() => setPreviewOpen(false)}><X size={17} /></button></div></div>
            <div className="document-preview-scroll">{previewLoading ? <div className="preview-loading">正在生成预览…</div> : null}{!previewLoading && markdownPreview !== null ? <article className="markdown-paper"><ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml>{markdownPreview}</ReactMarkdown></article> : null}</div>
          </section>
        </div>
      ), document.body) : null}
    </div>
  );
}

interface SectionHeadingProps {
  title: string;
  onCopy?: () => void;
}

/** 渲染总结区块标题及可选复制操作。 */
function SectionHeading({ title, onCopy }: SectionHeadingProps) {
  return <div className="minutes-heading"><h2>{title}</h2>{onCopy ? <button className="section-copy" type="button" onClick={onCopy}><Clipboard size={14} aria-hidden="true" />复制本节</button> : null}</div>;
}

interface StatementSectionProps {
  title: string;
  items: SupportedStatement[];
  onCopy: () => void;
}

/** 渲染结论、观点或决策等有序事实列表。 */
function StatementSection({ title, items, onCopy }: StatementSectionProps) {
  return (
    <section className="minutes-section">
      <SectionHeading title={title} onCopy={items.length > 0 ? onCopy : undefined} />
      {items.length > 0 ? <ul className="statement-list">{items.map((item, index) => <li key={`${item.content}-${index}`}><Check size={16} aria-hidden="true" /><span>{item.content}</span></li>)}</ul> : <p className="empty-copy"><AlertTriangle size={15} aria-hidden="true" />未提取到相关内容</p>}
    </section>
  );
}
