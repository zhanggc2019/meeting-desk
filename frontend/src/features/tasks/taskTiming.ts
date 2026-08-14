import type { ProcessingTask } from "../../contracts/desktop";

export type TaskTimingKind = "remaining" | "overdue" | "estimating" | "completed" | "unavailable";

export interface TaskTiming {
  kind: TaskTimingKind;
  totalEstimatedMs: number | null;
  remainingMs: number | null;
  estimatedCompletionAt: string | null;
  actualProcessingMs: number | null;
}

const TERMINAL_WITHOUT_RESULT = new Set<ProcessingTask["status"]>(["failed", "cancelled", "interrupted"]);

/** 汇总历史尝试和当前尝试已经消耗的实际处理时间。 */
function getElapsedProcessingMs(task: ProcessingTask, nowMs: number): number {
  const accumulatedMs = Math.max(0, task.processingDurationMs ?? 0);
  if (!task.processingStartedAt) return accumulatedMs;
  const startedAtMs = Date.parse(task.processingStartedAt);
  if (!Number.isFinite(startedAtMs)) return accumulatedMs;
  return accumulatedMs + Math.max(0, nowMs - startedAtMs);
}

/** 将任务记录转换为界面展示所需的预计剩余、完成时刻或真实耗时状态。 */
export function getTaskTiming(task: ProcessingTask, nowMs = Date.now()): TaskTiming {
  const emptyTiming = {
    totalEstimatedMs: null,
    remainingMs: null,
    estimatedCompletionAt: null,
    actualProcessingMs: null,
  };
  if (task.status === "completed") {
    return { ...emptyTiming, kind: "completed", actualProcessingMs: task.processingDurationMs ?? null };
  }
  if (TERMINAL_WITHOUT_RESULT.has(task.status)) {
    return { ...emptyTiming, kind: "unavailable" };
  }
  const totalEstimatedMs = task.estimatedProcessingMs ?? null;
  if (totalEstimatedMs === null || totalEstimatedMs <= 0) {
    return { ...emptyTiming, kind: "estimating" };
  }
  const remainingMs = totalEstimatedMs - getElapsedProcessingMs(task, nowMs);
  if (remainingMs <= 0) {
    return { ...emptyTiming, kind: "overdue", totalEstimatedMs };
  }
  return {
    kind: "remaining",
    totalEstimatedMs,
    remainingMs,
    estimatedCompletionAt: new Date(nowMs + remainingMs).toISOString(),
    actualProcessingMs: null,
  };
}

/** 将毫秒时长向上取整为适合预计时间的自然语言，避免伪装成精确秒数。 */
export function formatApproximateDuration(durationMs: number): string {
  if (durationMs < 60_000) return "不到 1 分钟";
  const totalMinutes = Math.ceil(durationMs / 60_000);
  if (totalMinutes < 60) return `约 ${totalMinutes} 分钟`;
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return minutes === 0 ? `约 ${hours} 小时` : `约 ${hours} 小时 ${minutes} 分钟`;
}

/** 将预计完成时间格式化为当前 Windows 本地时区的时分。 */
export function formatEstimatedCompletion(value: string): string {
  return new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit" }).format(new Date(value));
}
