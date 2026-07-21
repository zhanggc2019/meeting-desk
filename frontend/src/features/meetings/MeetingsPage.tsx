import { Clipboard, Download, FileSearch, Plus, Search } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { MeetingSummary } from "../../contracts/desktop";
import { useDesktopClient } from "../../services/DesktopClientContext";
import { getSafeErrorMessage } from "../../services/desktopClient";
import { useAppStore } from "../../stores/appStore";
import { formatDateTime, formatDuration } from "../../utils/format";

/** 安全地复制用户明确请求的本地会议文本。 */
async function copyText(value: string): Promise<void> {
  await navigator.clipboard.writeText(value);
}

/** 渲染仅查询本地存储的会议历史页面。 */
export function MeetingsPage() {
  const client = useDesktopClient();
  const navigate = useAppStore((state) => state.navigate);
  const openMeeting = useAppStore((state) => state.openMeeting);
  const [query, setQuery] = useState("");
  const [meetings, setMeetings] = useState<MeetingSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const requestSequence = useRef(0);

  useEffect(() => {
    const currentRequest = ++requestSequence.current;
    const timer = window.setTimeout(() => {
      setLoading(true);
      client.listMeetings(query)
        .then((items) => {
          if (currentRequest === requestSequence.current) {
            setMeetings(items);
            setError(null);
          }
        })
        .catch((reason: unknown) => currentRequest === requestSequence.current && setError(getSafeErrorMessage(reason)))
        .finally(() => currentRequest === requestSequence.current && setLoading(false));
    }, query ? 280 : 0);
    return () => window.clearTimeout(timer);
  }, [client, query]);

  /** 复制列表中已经保存的会议摘要。 */
  async function handleCopySummary(meeting: MeetingSummary) {
    if (!meeting.summary) return;
    try {
      await copyText(meeting.summary);
      setNotice("已复制摘要");
    } catch {
      setError("复制失败，请检查系统剪贴板权限");
    }
  }

  /** 调用桌面后端导出指定会议的 Markdown。 */
  async function handleExport(meetingId: string) {
    try {
      const result = await client.exportMeetingMarkdown(meetingId);
      if (result.status === "exported") setNotice("Markdown 已导出");
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    }
  }

  return (
    <div className="page meetings-page">
      <header className="page-header">
        <div>
          <span className="eyebrow">本地资料库</span>
          <h1 tabIndex={-1}>会议记录</h1>
          <p>搜索已保存的标题、摘要和会议内容，不发起云端请求。</p>
        </div>
        <button className="button primary" type="button" onClick={() => navigate("workspace")}><Plus size={16} aria-hidden="true" />选择媒体文件</button>
      </header>

      <div className="search-row">
        <label className="search-field">
          <Search size={17} aria-hidden="true" />
          <span className="visually-hidden">搜索会议记录</span>
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索标题或会议内容" type="search" />
        </label>
        <span className="result-count">{loading ? "正在搜索" : `${meetings.length} 条记录`}</span>
      </div>

      {notice ? <div className="toast" role="status">{notice}<button type="button" aria-label="关闭提示" onClick={() => setNotice(null)}>×</button></div> : null}
      {error ? <div className="inline-alert error" role="alert"><span>{error}</span><button type="button" onClick={() => setError(null)}>关闭</button></div> : null}

      {loading && meetings.length === 0 ? <div className="loading-state">正在读取会议记录…</div> : null}
      {!loading && meetings.length === 0 ? (
        <div className="empty-state">
          <FileSearch size={30} aria-hidden="true" />
          <h2>{query ? "没有找到匹配的会议" : "还没有会议记录"}</h2>
          <p>{query ? "尝试其他关键词，或清除当前搜索。" : "选择本地音频或视频，完成处理后会显示在这里。"}</p>
          {query ? <button className="button secondary" type="button" onClick={() => setQuery("")}>清除搜索</button> : <button className="button primary" type="button" onClick={() => navigate("workspace")}>选择媒体文件</button>}
        </div>
      ) : null}

      {meetings.length > 0 ? (
        <div className="table-wrap meeting-table-wrap">
          <table>
            <thead><tr><th>会议标题</th><th>会议时间</th><th>录音时长</th><th>总处理耗时</th><th>模板</th><th><span className="visually-hidden">操作</span></th></tr></thead>
            <tbody>
              {meetings.map((item) => (
                <tr key={item.id}>
                  <td><button className="meeting-title-link" type="button" onClick={() => openMeeting(item.id)}><strong>{item.title ?? "未命名会议"}</strong><span>{item.summary ?? "未提取到摘要"}</span></button></td>
                  <td>{formatDateTime(item.meetingStartAt)}</td>
                  <td>{formatDuration(item.durationMs)}</td>
                  <td>{formatDuration(item.processingDurationMs)}</td>
                  <td>{item.templateName}</td>
                  <td className="cell-actions">
                    <button className="icon-button" type="button" aria-label={`复制 ${item.title ?? "未命名会议"} 的摘要`} disabled={!item.summary} onClick={() => void handleCopySummary(item)}><Clipboard size={16} /></button>
                    <button className="icon-button" type="button" aria-label={`导出 ${item.title ?? "未命名会议"}`} onClick={() => void handleExport(item.id)}><Download size={16} /></button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
    </div>
  );
}
