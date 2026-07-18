import { ArrowRight, FileAudio, RefreshCw, RotateCcw, SlidersHorizontal, XCircle } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { ConfirmDialog } from "../../components/ui/ConfirmDialog";
import type { ProcessingTask, TaskQuery } from "../../contracts/desktop";
import { useDesktopClient } from "../../services/DesktopClientContext";
import { getSafeErrorMessage } from "../../services/desktopClient";
import { useAppStore } from "../../stores/appStore";
import { formatRelativeDate, getTaskStatusLabel } from "../../utils/format";

const filters: Array<{ id: TaskQuery["filter"]; label: string }> = [
  { id: "all", label: "全部" },
  { id: "active", label: "进行中" },
  { id: "failed", label: "失败" },
  { id: "completed", label: "已完成" },
];

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
  const [tasks, setTasks] = useState<ProcessingTask[]>([]);
  const [selectedTaskId, setSelectedTaskId] = useState<string | null>(null);
  const [cancelTaskId, setCancelTaskId] = useState<string | null>(null);
  const [busyTaskId, setBusyTaskId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const selectedTask = useMemo(() => tasks.find((task) => task.id === selectedTaskId) ?? tasks[0] ?? null, [selectedTaskId, tasks]);

  /** 从持久化任务源刷新当前筛选结果。 */
  async function loadTasks() {
    setLoading(true);
    setError(null);
    try {
      const nextTasks = await client.listProcessingTasks({ filter });
      setTasks(nextTasks);
      setTaskAttentionCount(nextTasks.filter((task) => ["failed", "interrupted"].includes(task.status)).length);
      if (selectedTaskId && !nextTasks.some((task) => task.id === selectedTaskId)) {
        setSelectedTaskId(null);
      }
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void loadTasks();
  }, [filter]);

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
          <button key={item.id} className="filter-button" type="button" aria-pressed={filter === item.id} onClick={() => setFilter(item.id)}>{item.label}</button>
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
              <p>选择本地音频文件后，处理状态会显示在这里。</p>
              <button className="button primary" type="button" onClick={() => navigate("workspace")}>选择音频文件</button>
            </div>
          ) : null}
          {tasks.length > 0 ? (
            <div className="table-wrap">
              <table className="task-table">
                <thead><tr><th>文件名</th><th>当前阶段</th><th>尝试</th><th>更新时间</th><th><span className="visually-hidden">操作</span></th></tr></thead>
                <tbody>
                  {tasks.map((task) => (
                    <tr key={task.id} className={selectedTask?.id === task.id ? "selected-row" : undefined} onClick={() => setSelectedTaskId(task.id)}>
                      <td><button className="table-row-link" type="button" onClick={() => setSelectedTaskId(task.id)}>{task.displayName}</button></td>
                      <td>
                        <span className={`status-label ${task.status}`}><span className="status-dot" aria-hidden="true" />{getTaskStatusLabel(task.status)}</span>
                        {task.progress !== null ? <progress value={task.progress} max={1} aria-label={`${task.displayName} 处理进度`} /> : null}
                      </td>
                      <td>{task.attempt} / {task.maxAttempts}</td>
                      <td><time dateTime={task.updatedAt}>{formatRelativeDate(task.updatedAt)}</time></td>
                      <td className="cell-actions">
                        {task.availableActions.includes("cancel") ? <button className="button table-action" type="button" onClick={(event) => { event.stopPropagation(); setCancelTaskId(task.id); }} disabled={busyTaskId === task.id}>取消</button> : null}
                        {task.availableActions.includes("retry") ? <button className="button table-action" type="button" onClick={(event) => { event.stopPropagation(); void retryTask(task.id); }} disabled={busyTaskId === task.id}><RotateCcw size={14} aria-hidden="true" />重试</button> : null}
                        {task.availableActions.includes("reselectFile") ? <button className="button table-action" type="button" onClick={(event) => { event.stopPropagation(); void reselectTaskAudio(task.id); }} disabled={busyTaskId === task.id}>重新选择</button> : null}
                        {task.availableActions.includes("openMeeting") && task.meetingId ? <button className="button table-action" type="button" onClick={(event) => { event.stopPropagation(); openMeeting(task.meetingId!); }}>查看</button> : null}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : null}
        </section>

        {selectedTask ? <TaskInspector task={selectedTask} onOpenMeeting={openMeeting} onOpenSettings={() => useAppStore.getState().openSettings()} /> : null}
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
    </div>
  );
}

interface TaskInspectorProps {
  task: ProcessingTask;
  onOpenMeeting: (meetingId: string) => void;
  onOpenSettings: () => void;
}

/** 在右侧检查器中展示任务的真实阶段与安全错误。 */
function TaskInspector({ task, onOpenMeeting, onOpenSettings }: TaskInspectorProps) {
  const currentIndex = timelineStages.indexOf(task.status as (typeof timelineStages)[number]);
  return (
    <aside className="task-inspector" aria-label={`${task.displayName} 任务详情`}>
      <div className="inspector-heading">
        <span className="eyebrow">任务详情</span>
        <h2>{task.displayName}</h2>
        <p>{getTaskStatusLabel(task.status)}</p>
      </div>

      {task.error ? (
        <div className="task-error" role="status">
          <XCircle size={18} aria-hidden="true" />
          <div><strong>{task.error.safeMessage}</strong><small>{task.error.code}</small></div>
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
