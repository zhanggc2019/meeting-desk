import { ArrowRight, Clock3, FileAudio, RefreshCw, RotateCcw, SlidersHorizontal, Trash2, XCircle } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { ConfirmDialog } from "../../components/ui/ConfirmDialog";
import { Pagination } from "../../components/ui/Pagination";
import type { ProcessingTask, TaskQuery } from "../../contracts/desktop";
import { useDesktopClient } from "../../services/DesktopClientContext";
import { getSafeErrorMessage } from "../../services/desktopClient";
import { useAppStore } from "../../stores/appStore";
import { formatDuration, formatRelativeDate, getTaskStatusLabel } from "../../utils/format";
import { formatApproximateDuration, formatEstimatedCompletion, getTaskTiming } from "./taskTiming";

const filters: Array<{ id: TaskQuery["filter"]; label: string }> = [
  { id: "all", label: "全部" },
  { id: "active", label: "进行中" },
  { id: "failed", label: "失败" },
  { id: "completed", label: "已完成" },
];

const TASKS_PAGE_SIZE = 10;

const timelineStages = [
  "queued",
  "preparing",
  "uploading",
  "transcribing",
  "validating_transcript",
  "summarizing",
  "validating_minutes",
  "saving",
  "completed",
] as const;

/** 渲染可取消、可重试并能查看真实阶段的任务队列。 */
export function TasksPage() {
  const client = useDesktopClient();
  const navigate = useAppStore((state) => state.navigate);
  const openMeeting = useAppStore((state) => state.openMeeting);
  const setTaskAttentionCount = useAppStore((state) => state.setTaskAttentionCount);
  const [filter, setFilter] = useState<TaskQuery["filter"]>("all");
  const [page, setPage] = useState(1);
  const [total, setTotal] = useState(0);
  const [totalPages, setTotalPages] = useState(1);
  const [tasks, setTasks] = useState<ProcessingTask[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [cancelTaskId, setCancelTaskId] = useState<string | null>(null);
  const [deleteTaskId, setDeleteTaskId] = useState<string | null>(null);
  const [busyTaskId, setBusyTaskId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const selectedTask = useMemo(() => tasks.find((task) => task.id === selectedTaskId) ?? tasks[0] ?? null, [selectedTaskId, tasks]);
  const deleteTarget = useMemo(() => tasks.find((task) => task.id === deleteTaskId) ?? null, [deleteTaskId, tasks]);
  const hasActiveTasks = useMemo(
    () => tasks.some((task) => !["completed", "failed", "cancelled", "interrupted"].includes(task.status)),
    [tasks],
  );
  const nowMs = Date.now();

  /** 从持久化任务源刷新当前筛选结果。 */
  const loadTasks = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [result, attentionTasks] = await Promise.all([
        client.listProcessingTasksPage({ filter, page, pageSize: TASKS_PAGE_SIZE }),
        client.listProcessingTasks({ filter: "failed" }),
      ]);
      setTasks(result.items);
      setTotal(result.total);
      setTotalPages(result.totalPages);
      setPage((current) => current === result.page ? current : result.page);
      setTaskAttentionCount(attentionTasks.length);
      setSelectedTaskId((currentId) => currentId && !result.items.some((task) => task.id === currentId) ? null : currentId);
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setLoading(false);
    }
  }, [client, filter, page, setTaskAttentionCount]);

  useEffect(() => {
    void loadTasks();
  }, [loadTasks]);

  /** 仅在存在进行中任务时定期刷新队列，并在离开页面时停止。 */
  useEffect(() => {
    if (!hasActiveTasks) return undefined;
    const intervalId = window.setInterval(() => void loadTasks(), 3_000);
    return () => window.clearInterval(intervalId);
  }, [hasActiveTasks, loadTasks]);

  /** 确认并提交单个任务取消请求。 */
  async function confirmCancelTask() {
    if (!cancelTaskId || busyTaskId) return;
    setBusyTaskId(cancelTaskId);
    try {
      await client.cancelProcessingTask(cancelTaskId);
      setCancelTaskId(null);
      await loadTasks();
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setBusyTaskId(null);
    }
  }

  /** 依据后端允许动作重新提交一个失败任务。 */
  async function retryTask(taskId: string) {
    if (busyTaskId) return;
    setBusyTaskId(taskId);
    setError(null);
    try {
      await client.retryProcessingTask(taskId);
      await loadTasks();
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setBusyTaskId(null);
    }
  }

  /** 确认清除终态任务，并由后端级联清理该任务拥有的本地资料。 */
  async function confirmDeleteTask() {
    if (!deleteTaskId || busyTaskId) return;
    setBusyTaskId(deleteTaskId);
    setError(null);
    try {
      await client.deleteProcessingTask(deleteTaskId);
      setDeleteTaskId(null);
      await loadTasks();
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setBusyTaskId(null);
    }
  }

  /** 通过系统文件对话框重新绑定音频并续接重启后中断的任务。 */
  async function reselectTaskAudio(taskId: string) {
    if (busyTaskId) return;
    setBusyTaskId(taskId);
    setError(null);
    try {
      await client.reselectProcessingTask(taskId);
      await loadTasks();
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setBusyTaskId(null);
    }
  }

  /** 切换任务筛选并从第一页重新查询。 */
  function changeFilter(nextFilter: TaskQuery["filter"]) {
    setFilter(nextFilter);
    setPage(1);
    setSelectedTaskId(null);
  }

  /** 切换任务页码并清除上一页的检查器选择。 */
  function changePage(nextPage: number) {
    setPage(nextPage);
    setSelectedTaskId(null);
  }

  return (
    <div className="page tasks-page">
      <header className="page-header">
        <div>
          <span className="eyebrow">处理状态</span>
          <h1 tabIndex={-1}>任务队列</h1>
          <p>每个文件独立处理；取消或失败不会影响同批其他任务。</p>
        </div>
        <button className="button secondary" type="button" onClick={() => void loadTasks()} disabled={loading}>
          <RefreshCw size={16} aria-hidden="true" />刷新
        </button>
      </header>

      <div className="filter-bar" aria-label="任务筛选">
        <SlidersHorizontal size={16} aria-hidden="true" />
        {filters.map((item) => (
          <button key={item.id} className="filter-button" type="button" aria-pressed={filter === item.id} onClick={() => changeFilter(item.id)}>{item.label}</button>
        ))}
      </div>

      {error ? <div className="inline-alert error" role="alert"><span>{error}</span><button type="button" onClick={() => setError(null)}>关闭</button></div> : null}

      <div className={`task-layout${selectedTask ? " has-inspector" : ""}`}>
        <section className="task-list-panel" aria-label="处理任务列表">
          {loading && tasks.length === 0 ? <div className="loading-state">正在读取任务…</div> : null}
          {!loading && tasks.length === 0 ? (
            <div className="empty-state compact-empty">
              <FileAudio size={26} aria-hidden="true" />
              <h2>当前没有匹配的任务</h2>
              <p>选择本地音频或视频后，处理状态会显示在这里。</p>
              <button className="button primary" type="button" onClick={() => navigate("workspace")}>选择媒体文件</button>
            </div>
          ) : null}
          {tasks.length > 0 ? (
            <div className="table-wrap">
              <table className="task-table">
                <thead><tr><th>文件名</th><th>当前阶段</th><th>预计剩余</th><th>尝试</th><th>更新时间</th><th><span className="visually-hidden">操作</span></th></tr></thead>
                <tbody>
                  {tasks.map((task) => (
                    <tr key={task.id} className={selectedTask?.id === task.id ? "selected-row" : undefined} onClick={() => setSelectedTaskId(task.id)}>
                      <td><button className="table-row-link" type="button" onClick={() => setSelectedTaskId(task.id)}>{task.displayName}</button></td>
                      <td>
                        <span className={`status-label ${task.status}`}><span className="status-dot" aria-hidden="true" />{getTaskStatusLabel(task.status)}</span>
                        {task.progress !== null ? <progress value={task.progress} max={1} aria-label={`${task.displayName} 处理进度`} /> : null}
                      </td>
                      <td className="estimated-time-cell"><TaskTimingCell task={task} nowMs={nowMs} /></td>
                      <td>{task.attempt} / {task.maxAttempts}</td>
                      <td><time dateTime={task.updatedAt}>{formatRelativeDate(task.updatedAt)}</time></td>
                      <td className="cell-actions">
                        {task.availableActions.includes("cancel") ? <button className="button table-action" type="button" onClick={(event) => { event.stopPropagation(); setCancelTaskId(task.id); }} disabled={busyTaskId === task.id}>取消</button> : null}
                        {task.availableActions.includes("retry") ? <button className="button table-action" type="button" onClick={(event) => { event.stopPropagation(); void retryTask(task.id); }} disabled={busyTaskId === task.id}><RotateCcw size={14} aria-hidden="true" />重试</button> : null}
                        {task.availableActions.includes("delete") ? <button className="button table-action delete-action" type="button" onClick={(event) => { event.stopPropagation(); setDeleteTaskId(task.id); }} disabled={busyTaskId === task.id}><Trash2 size={14} aria-hidden="true" />{task.status === "completed" ? "清除" : "删除"}</button> : null}
                        {task.availableActions.includes("reselectFile") ? <button className="button table-action" type="button" onClick={(event) => { event.stopPropagation(); void reselectTaskAudio(task.id); }} disabled={busyTaskId === task.id}>重新选择</button> : null}
                        {task.availableActions.includes("openMeeting") && task.meetingId ? <button className="button table-action" type="button" onClick={(event) => { event.stopPropagation(); openMeeting(task.meetingId!); }}>查看</button> : null}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
          {total > 0 ? <Pagination page={page} totalPages={totalPages} total={total} disabled={loading} onPageChange={changePage} /> : null}
        </section>

        {selectedTask ? <TaskInspector task={selectedTask} nowMs={nowMs} retrying={busyTaskId === selectedTask.id} onRetry={() => void retryTask(selectedTask.id)} onOpenMeeting={openMeeting} onOpenSettings={() => useAppStore.getState().openSettings()} /> : null}
      </div>

      <ConfirmDialog
        open={cancelTaskId !== null}
        title="取消处理任务？"
        description="取消后将停止后续处理。已发送到云端的请求可能继续执行或产生用量。"
        confirmLabel="取消任务"
        busy={busyTaskId === cancelTaskId}
        onCancel={() => setCancelTaskId(null)}
        onConfirm={() => void confirmCancelTask()}
      />
      <ConfirmDialog
        open={deleteTaskId !== null}
        title={deleteTarget?.status === "completed" ? "清除已完成任务？" : "删除失败任务？"}
        description={deleteTarget?.status === "completed"
          ? "清除后，关联会议记录、逐字稿、会议纪要和本地任务数据都会从本机永久删除；你导入的原始媒体文件不会受影响。"
          : "只会删除任务记录和受管临时文件，不会删除你导入的原始文件。"}
        confirmLabel={deleteTarget?.status === "completed" ? "确认清除" : "删除任务"}
        busy={busyTaskId === deleteTaskId}
        onCancel={() => setDeleteTaskId(null)}
        onConfirm={() => void confirmDeleteTask()}
      />
    </div>
  );
}

interface TaskInspectorProps {
  task: ProcessingTask;
  nowMs: number;
  retrying: boolean;
  onRetry: () => void;
  onOpenMeeting: (meetingId: string) => void;
  onOpenSettings: () => void;
}

/** 在右侧检查器中展示任务的真实阶段与安全错误。 */
function TaskInspector({ task, nowMs, retrying, onRetry, onOpenMeeting, onOpenSettings }: TaskInspectorProps) {
  const currentIndex = timelineStages.indexOf(task.status as (typeof timelineStages)[number]);
  return (
    <aside className="task-inspector" aria-label={`${task.displayName} 任务详情`}>
      <div className="inspector-heading">
        <span className="eyebrow">任务详情</span>
        <h2>{task.displayName}</h2>
        <p>{getTaskStatusLabel(task.status)}</p>
      </div>

      <TaskTimingSummary task={task} nowMs={nowMs} />

      {task.error ? (
        <div className="task-error" role="status">
          <XCircle size={18} aria-hidden="true" />
          <div><strong>{task.error.safeMessage}</strong><small>{task.error.code}</small></div>
          {task.availableActions.includes("retry") ? <button className="button quiet" type="button" onClick={onRetry} disabled={retrying}><RotateCcw size={14} aria-hidden="true" />{retrying ? "正在重试" : "重试任务"}</button> : null}
          {[401, 403].includes(task.error.httpStatus ?? 0) ? <button className="button quiet" type="button" onClick={onOpenSettings}>前往设置</button> : null}
        </div>
      ) : null}

      <ol className="task-timeline">
        {timelineStages.map((stage, index) => {
          const reached = task.status === "completed" || (currentIndex >= 0 && index <= currentIndex);
          const current = stage === task.status;
          return (
            <li key={stage} className={`${reached ? "reached" : "pending"}${current ? " current" : ""}`}>
              <span className="timeline-marker" aria-hidden="true" />
              <span>{getTaskStatusLabel(stage)}</span>
            </li>
          );
        })}
      </ol>

      {task.meetingId ? (
        <button className="button primary inspector-action" type="button" onClick={() => onOpenMeeting(task.meetingId!)}>
          查看会议纪要<ArrowRight size={16} aria-hidden="true" />
        </button>
      ) : null}
    </aside>
  );
}

interface TaskTimingProps {
  task: ProcessingTask;
  nowMs: number;
}

/** 在任务表格中显示紧凑的预计剩余时间或真实处理耗时。 */
function TaskTimingCell({ task, nowMs }: TaskTimingProps) {
  const timing = getTaskTiming(task, nowMs);
  if (timing.kind === "remaining") return <>还需{formatApproximateDuration(timing.remainingMs!)}</>;
  if (timing.kind === "overdue") return <>已超出预计</>;
  if (timing.kind === "estimating") return <>正在估算</>;
  if (timing.kind === "completed") return <>实际 {formatDuration(timing.actualProcessingMs)}</>;
  return <>—</>;
}

/** 在任务详情中解释预计总耗时、剩余时间和估算依据。 */
function TaskTimingSummary({ task, nowMs }: TaskTimingProps) {
  const timing = getTaskTiming(task, nowMs);
  if (timing.kind === "unavailable") return null;
  if (timing.kind === "completed") {
    return (
      <div className="task-timing-summary completed">
        <Clock3 size={18} aria-hidden="true" />
        <div><strong>实际处理耗时 {formatDuration(timing.actualProcessingMs)}</strong><small>任务已完成</small></div>
      </div>
    );
  }
  if (timing.kind === "estimating") {
    return (
      <div className="task-timing-summary">
        <Clock3 size={18} aria-hidden="true" />
        <div><strong>正在估算</strong><small>读取不到录音时长时，将在获得更多信息后更新</small></div>
      </div>
    );
  }
  if (timing.kind === "overdue") {
    return (
      <div className="task-timing-summary overdue">
        <Clock3 size={18} aria-hidden="true" />
        <div><strong>已超出初始预计</strong><small>任务仍在处理中，实际耗时会受音频内容和本机负载影响</small></div>
      </div>
    );
  }
  return (
    <div className="task-timing-summary">
      <Clock3 size={18} aria-hidden="true" />
      <div>
        <strong>预计还需{formatApproximateDuration(timing.remainingMs!)}</strong>
        <small>预计总耗时{formatApproximateDuration(timing.totalEstimatedMs!)} · 预计 {formatEstimatedCompletion(timing.estimatedCompletionAt!)} 完成</small>
        <small>根据录音时长和本机历史处理速度动态估算</small>
      </div>
    </div>
  );
}
