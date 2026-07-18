import type { TaskStatus } from "../contracts/desktop";

const taskStatusLabels: Record<TaskStatus, string> = {
  queued: "等待处理",
  preparing: "正在检查文件",
  uploading: "正在上传",
  transcribing: "正在转写",
  validating_transcript: "正在校验转写结果",
  summarizing: "正在生成会议纪要",
  validating_minutes: "正在校验会议纪要",
  saving: "正在保存",
  retry_wait: "等待重试",
  cancel_requested: "正在取消",
  interrupted: "处理已中断",
  completed: "已完成",
  failed: "处理失败",
  cancelled: "已取消",
};

/** 返回统一的任务状态中文文案。 */
export function getTaskStatusLabel(status: TaskStatus): string {
  return taskStatusLabels[status];
}

/** 将可选字节数格式化为紧凑的文件大小。 */
export function formatBytes(bytes: number | null): string {
  if (bytes === null) return "—";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/** 将可选毫秒时长格式化为时分秒。 */
export function formatDuration(durationMs: number | null | undefined): string {
  if (durationMs === null || durationMs === undefined) return "—";
  const totalSeconds = Math.floor(durationMs / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  return hours > 0
    ? `${String(hours).padStart(2, "0")}:${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`
    : `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

/** 将 UTC 时间格式化为当前系统本地日期时间。 */
export function formatDateTime(value: string | null): string {
  if (!value) return "时间未提供";
  return new Intl.DateTimeFormat("zh-CN", { year: "numeric", month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(new Date(value));
}

/** 将更新时间格式化为适合任务列表扫描的短日期。 */
export function formatRelativeDate(value: string): string {
  return new Intl.DateTimeFormat("zh-CN", { month: "2-digit", day: "2-digit", hour: "2-digit", minute: "2-digit" }).format(new Date(value));
}

/** 将段落时间戳格式化为方括号时分秒。 */
export function formatTimestamp(value: number | undefined): string {
  if (value === undefined) return "";
  return `[${formatDuration(value)}]`;
}
