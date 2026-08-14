import { Clipboard, Download, FileSearch, Plus, Search, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ConfirmDialog } from "../../components/ui/ConfirmDialog";
import { Pagination } from "../../components/ui/Pagination";
import type { MeetingSummary } from "../../contracts/desktop";
import { useDesktopClient } from "../../services/DesktopClientContext";
import { getSafeErrorMessage } from "../../services/desktopClient";
import { useAppStore } from "../../stores/appStore";
import { formatDateTime, formatDuration } from "../../utils/format";

const MEETINGS_PAGE_SIZE = 10;

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
  const [page, setPage] = useState(1);
  const [total, setTotal] = useState(0);
  const [totalPages, setTotalPages] = useState(1);
  const [meetings, setMeetings] = useState<MeetingSummary[]>([]);
  const [deleteTarget, setDeleteTarget] = useState<MeetingSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const requestSequence = useRef(0);

  /** 从桌面端分页读取当前搜索条件下的会议摘要。 */
  const loadMeetings = useCallback(async () => {
    const currentRequest = ++requestSequence.current;
    setLoading(true);
    try {
      const result = await client.listMeetingsPage({ query, page, pageSize: MEETINGS_PAGE_SIZE });
      if (currentRequest !== requestSequence.current) return;
      setMeetings(result.items);
      setTotal(result.total);
      setTotalPages(result.totalPages);
      setPage((current) => current === result.page ? current : result.page);
      setError(null);
    } catch (reason) {
      if (currentRequest === requestSequence.current) setError(getSafeErrorMessage(reason));
    } finally {
      if (currentRequest === requestSequence.current) setLoading(false);
    }
  }, [client, page, query]);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      void loadMeetings();
    }, query ? 280 : 0);
    return () => window.clearTimeout(timer);
  }, [loadMeetings, query]);

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

  /** 更新搜索词并从第一页重新查询。 */
  function handleQueryChange(nextQuery: string) {
    setQuery(nextQuery);
    setPage(1);
  }

  /** 二次确认后删除会议及关联资料，再刷新当前分页。 */
  async function confirmDeleteMeeting() {
    if (!deleteTarget || deleting) return;
    setDeleting(true);
    setError(null);
    try {
      const deleted = await client.deleteMeeting(deleteTarget.id);
      if (!deleted) {
        setError("会议记录不存在或已被删除");
        return;
      }
      setDeleteTarget(null);
      setNotice("会议记录及关联资料已删除");
      await loadMeetings();
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setDeleting(false);
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
          <input value={query} onChange={(event) => handleQueryChange(event.target.value)} placeholder="搜索标题或会议内容" type="search" />
        </label>
        <span className="result-count">{loading ? "正在搜索" : `${total} 条记录`}</span>
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
                    <button className="icon-button delete-icon-action" type="button" aria-label={`删除 ${item.title ?? "未命名会议"}`} title="删除会议" onClick={() => setDeleteTarget(item)}><Trash2 size={16} /></button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : null}
      {total > 0 ? <Pagination page={page} totalPages={totalPages} total={total} disabled={loading} onPageChange={setPage} /> : null}

      <ConfirmDialog
        open={deleteTarget !== null}
        title="删除会议记录？"
        description="删除后，会议记录、逐字稿、会议纪要和关联任务都会从本机永久清理；你导入的原始媒体文件不会受影响。"
        confirmLabel="删除会议"
        busy={deleting}
        onCancel={() => setDeleteTarget(null)}
        onConfirm={() => void confirmDeleteMeeting()}
      />
    </div>
  );
}
