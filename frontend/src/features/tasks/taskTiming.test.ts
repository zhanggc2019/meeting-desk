import { describe, expect, it } from "vitest";
import type { ProcessingTask } from "../../contracts/desktop";
import { getTaskTiming } from "./taskTiming";

const activeTask: ProcessingTask = {
  id: "task-estimate",
  artifactId: "artifact-estimate",
  batchId: null,
  displayName: "预计耗时测试.wav",
  meetingId: null,
  templateId: "standard_meeting",
  status: "transcribing",
  attempt: 1,
  maxAttempts: 3,
  progress: 0.28,
  createdAt: "2026-08-14T02:00:00Z",
  updatedAt: "2026-08-14T02:10:00Z",
  processingStartedAt: "2026-08-14T02:00:00Z",
  processingDurationMs: 300_000,
  sourceDurationMs: 3_600_000,
  estimatedProcessingMs: 2_700_000,
  error: null,
  availableActions: ["cancel"],
};

describe("任务预计耗时", () => {
  it("累计历史尝试与当前尝试耗时后返回剩余时间和完成时刻", () => {
    const timing = getTaskTiming(activeTask, Date.parse("2026-08-14T02:10:00Z"));

    expect(timing.kind).toBe("remaining");
    expect(timing.remainingMs).toBe(1_800_000);
    expect(timing.estimatedCompletionAt).toBe("2026-08-14T02:40:00.000Z");
  });

  it("超过初始估算后提示耗时超出预期而不是显示零秒", () => {
    const timing = getTaskTiming(activeTask, Date.parse("2026-08-14T03:00:00Z"));

    expect(timing.kind).toBe("overdue");
    expect(timing.remainingMs).toBeNull();
    expect(timing.estimatedCompletionAt).toBeNull();
  });

  it("完成任务只显示真实处理耗时", () => {
    const timing = getTaskTiming(
      {
        ...activeTask,
        status: "completed",
        processingStartedAt: null,
        processingDurationMs: 2_640_000,
      },
      Date.parse("2026-08-14T03:00:00Z"),
    );

    expect(timing.kind).toBe("completed");
    expect(timing.actualProcessingMs).toBe(2_640_000);
    expect(timing.estimatedCompletionAt).toBeNull();
  });

  it("缺少估算时长的活动任务显示正在估算", () => {
    const timing = getTaskTiming(
      { ...activeTask, estimatedProcessingMs: null },
      Date.parse("2026-08-14T02:10:00Z"),
    );

    expect(timing.kind).toBe("estimating");
    expect(timing.remainingMs).toBeNull();
  });

  it("失败和取消任务不显示预计完成时间", () => {
    for (const status of ["failed", "cancelled", "interrupted"] as const) {
      const timing = getTaskTiming({ ...activeTask, status }, Date.parse("2026-08-14T02:10:00Z"));
      expect(timing.kind).toBe("unavailable");
      expect(timing.estimatedCompletionAt).toBeNull();
    }
  });
});
