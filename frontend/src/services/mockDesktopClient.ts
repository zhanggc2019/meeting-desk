import type {
  BrowserFileDescriptor,
  ImportCandidate,
  MeetingDetail,
  MeetingSummary,
  MinutesTemplate,
  ProcessingTask,
  PublicSettings,
  SaveProviderSettingsInput,
  TaskQuery,
} from "../contracts/desktop";
import type { DesktopClient } from "./desktopClient";

const templates: MinutesTemplate[] = [
  { id: "adaptive", version: "1.0.0", name: "自适应模板", description: "由大模型判断内容类型，并选择最合适的结构输出" },
  { id: "standard_meeting", version: "1.0.0", name: "标准会议纪要", description: "平衡摘要、结论、待办与风险" },
  { id: "project_weekly", version: "1.0.0", name: "项目周会", description: "突出进展、阻塞与明确后续动作" },
  { id: "customer_communication", version: "1.0.0", name: "客户沟通", description: "区分客户诉求、确认事项与待确认问题" },
  { id: "course_summary", version: "1.0.0", name: "课程总结", description: "提炼知识框架、核心概念、案例与复习要点" },
  { id: "research_project", version: "1.0.0", name: "课题研究", description: "聚焦研究问题、方法、证据、进展与下一步" },
  { id: "academic_lecture", version: "1.0.0", name: "学术讲座", description: "整理主题脉络、关键论点、研究发现与启发" },
  { id: "profile_interview", version: "1.0.0", name: "人物专访", description: "保留人物经历、观点、故事线与代表性表达" },
  { id: "in_depth_interview", version: "1.0.0", name: "深度访谈", description: "按议题组织问答、洞察、矛盾点与待核实信息" },
  { id: "business_plan", version: "1.0.0", name: "商业计划书", description: "梳理市场机会、方案、商业模式、里程碑与风险" },
  { id: "article_outline", version: "1.0.0", name: "文章大纲", description: "生成标题方向、章节层级、核心论据与素材提示" },
];

const meeting: MeetingDetail = {
  id: "meeting-demo-1",
  templateName: "标准会议纪要",
  durationMs: 2_430_000,
  createdAt: "2026-07-17T06:30:00Z",
  minutes: {
    schemaVersion: "1.0.0",
    title: "产品交付节奏讨论",
    titleSource: "generated",
    meetingTime: { startAt: null, endAt: null },
    participants: [],
    summary: "团队核对了当前交付范围，确认先完成核心流程验证，再安排扩展能力。",
    topics: [
      { title: "交付范围", summary: "首轮交付聚焦文件导入、转写和纪要闭环。", evidenceSegmentIds: ["segment-1"] },
      { title: "质量验证", summary: "在扩大范围前先完成错误与取消路径验证。", evidenceSegmentIds: ["segment-2"] },
    ],
    conclusions: [{ content: "首轮交付以可验证的核心闭环为准。", evidenceSegmentIds: ["segment-1"] }],
    decisions: [{ content: "本周先完成 Mock 全流程联调。", evidenceSegmentIds: ["segment-2"] }],
    actionItems: [
      {
        description: "整理完整验收清单",
        owner: "测试团队",
        dueDateText: "下周五",
        dueDate: null,
        evidenceSegmentIds: ["segment-3"],
      },
    ],
    risksAndIssues: [
      {
        kind: "risk",
        description: "真实服务字段仍需验证",
        impact: "可能影响最终适配周期",
        mitigation: "先保持 Provider 抽象并使用 Mock 联调",
        evidenceSegmentIds: ["segment-4"],
      },
    ],
  },
  transcript: {
    schemaVersion: "1",
    text: "我们先聚焦核心闭环。Mock 流程完成后，再接入真实服务。测试团队整理验收清单。真实服务字段仍需验证。",
    language: "zh-CN",
    durationMs: 2_430_000,
    segments: [
      { id: "segment-1", startMs: 0, endMs: 15_000, speakerLabel: "说话人 A", text: "我们先聚焦核心闭环。" },
      { id: "segment-2", startMs: 15_000, endMs: 34_000, speakerLabel: "说话人 B", text: "Mock 流程完成后，再接入真实服务。" },
      { id: "segment-3", startMs: 34_000, endMs: 46_000, speakerLabel: "说话人 A", text: "测试团队整理验收清单。" },
      { id: "segment-4", startMs: 46_000, endMs: 58_000, speakerLabel: "说话人 B", text: "真实服务字段仍需验证。" },
    ],
  },
};

const initialTasks: ProcessingTask[] = [
  {
    id: "task-active-1",
    artifactId: "artifact-active-1",
    batchId: null,
    displayName: "季度复盘.wav",
    meetingId: null,
    templateId: "standard_meeting",
    status: "transcribing",
    attempt: 1,
    maxAttempts: 3,
    progress: null,
    createdAt: "2026-07-17T06:20:00Z",
    updatedAt: "2026-07-17T06:42:00Z",
    error: null,
    availableActions: ["cancel"],
  },
  {
    id: "task-failed-1",
    artifactId: "artifact-failed-1",
    batchId: null,
    displayName: "客户访谈.mp3",
    meetingId: null,
    templateId: "customer_communication",
    status: "failed",
    attempt: 1,
    maxAttempts: 3,
    progress: null,
    createdAt: "2026-07-17T05:10:00Z",
    updatedAt: "2026-07-17T05:30:00Z",
    error: { code: "network_unavailable", safeMessage: "网络不可用，请检查连接后重试", retryable: true },
    availableActions: ["retry"],
  },
  {
    id: "task-complete-1",
    artifactId: "artifact-complete-1",
    batchId: null,
    displayName: "产品讨论.m4a",
    meetingId: meeting.id,
    templateId: "standard_meeting",
    status: "completed",
    attempt: 1,
    maxAttempts: 3,
    progress: 1,
    createdAt: "2026-07-17T04:30:00Z",
    updatedAt: "2026-07-17T04:52:00Z",
    error: null,
    availableActions: ["openMeeting"],
  },
];

const initialSettings: PublicSettings = {
  transcription: {
    presetId: "mock",
    kind: "mock",
    endpoint: "",
    model: "mock-asr",
    secretConfigured: false,
    connectTimeoutMs: 10_000,
    requestTimeoutMs: 120_000,
    maxRetries: 2,
  },
  minutes: {
    presetId: "mock",
    kind: "mock",
    endpoint: "",
    model: "mock-minutes",
    secretConfigured: false,
    connectTimeoutMs: 10_000,
    requestTimeoutMs: 120_000,
    maxRetries: 2,
  },
};

/** 生成与 Mock 导出结果一致的 Markdown 预览文本。 */
function createMeetingMarkdown(): string {
  return `# 产品交付节奏讨论

> 标准会议纪要 · 40 分 30 秒

## 会议摘要

团队核对了当前交付范围，确认先完成核心流程验证，再安排扩展能力。

## 主要议题

1. **交付范围**：首轮交付聚焦文件导入、转写和纪要闭环。
2. **质量验证**：在扩大范围前先完成错误与取消路径验证。

## 待办事项

| 事项 | 负责人 | 截止日期 |
| --- | --- | --- |
| 整理完整验收清单 | 测试团队 | 下周五 |

## 完整逐字稿

我们先聚焦核心闭环。Mock 流程完成后，再接入真实服务。`;
}

/** 根据扩展名返回供 Mock 校验使用的 MIME。 */
function inferMimeType(name: string, providedType: string): string | null {
  if (providedType.startsWith("audio/")) {
    return providedType;
  }
  const extension = name.split(".").pop()?.toLowerCase();
  const mimeByExtension: Record<string, string> = {
    mp3: "audio/mpeg",
    wav: "audio/wav",
    m4a: "audio/mp4",
  };
  return extension ? (mimeByExtension[extension] ?? null) : null;
}

/** 创建只含安全显示信息的 Mock 导入候选项。 */
function createCandidate(file: BrowserFileDescriptor, index: number): ImportCandidate {
  const mimeType = inferMimeType(file.name, file.type);
  const isEmpty = file.size === 0;
  const isSupported = mimeType !== null;
  return {
    id: `candidate-${Date.now()}-${index}`,
    artifactId: !isEmpty && isSupported ? `artifact-${Date.now()}-${index}` : null,
    displayName: file.name,
    mimeType,
    sizeBytes: file.size,
    durationMs: !isEmpty && isSupported ? 1_860_000 : null,
    validationStatus: !isEmpty && isSupported ? "ready" : "invalid",
    safeMessage: isEmpty ? "文件为空" : isSupported ? null : "格式不受支持",
  };
}

/** 仅在供应商预设未变化时沿用已保存密钥状态。 */
function resolveMockSecretStatus(
  currentPresetId: PublicSettings["transcription"]["presetId"],
  nextPresetId: SaveProviderSettingsInput["transcription"]["presetId"],
  currentConfigured: boolean,
  nextSecret: string | undefined,
): boolean {
  return Boolean(nextSecret) || (currentPresetId === nextPresetId && currentConfigured);
}

/** 返回一个完全离线、确定性的前端 Mock 客户端。 */
export function createMockDesktopClient(): DesktopClient {
  let tasks = initialTasks.map((task) => ({ ...task, availableActions: [...task.availableActions] }));
  let settings = structuredClone(initialSettings);
  let sequence = 1;
  const artifactNames = new Map<string, string>();

  return {
    async selectAudioFiles(mode) {
      const mockFiles =
        mode === "batch"
          ? [
              { name: "演示会议.mp3", size: 12_582_912, type: "audio/mpeg" },
              { name: "客户访谈.m4a", size: 8_388_608, type: "audio/mp4" },
            ]
          : [{ name: "演示会议.mp3", size: 12_582_912, type: "audio/mpeg" }];
      const candidates = mockFiles.map((file, index) => createCandidate(file, sequence + index));
      sequence += mockFiles.length;
      candidates.forEach((candidate) => {
        if (candidate.artifactId) artifactNames.set(candidate.artifactId, candidate.displayName);
      });
      return candidates;
    },
    async registerBrowserFiles(files) {
      const candidates = files.map((file, index) => createCandidate(file, sequence + index));
      sequence += files.length;
      candidates.forEach((candidate) => {
        if (candidate.artifactId) artifactNames.set(candidate.artifactId, candidate.displayName);
      });
      return candidates;
    },
    async releaseAudioArtifact(artifactId) {
      artifactNames.delete(artifactId);
    },
    async listMinutesTemplates() {
      return structuredClone(templates);
    },
    async createProcessingTasks(artifactIds, templateId) {
      const now = new Date().toISOString();
      const created = artifactIds.map((artifactId, index) => ({
        id: `task-created-${sequence++}`,
        artifactId,
        batchId: artifactIds.length > 1 ? `batch-${now}` : null,
        displayName: artifactNames.get(artifactId) ?? `待处理文件 ${index + 1}`,
        meetingId: null,
        templateId,
        status: "queued" as const,
        attempt: 0,
        maxAttempts: 3,
        progress: null,
        createdAt: now,
        updatedAt: now,
        error: null,
        availableActions: ["cancel" as const],
      }));
      tasks = [...created, ...tasks];
      return structuredClone(created);
    },
    async listProcessingTasks(query: TaskQuery) {
      const filtered = tasks.filter((task) => {
        if (query.filter === "active") {
          return !["completed", "failed", "cancelled"].includes(task.status);
        }
        if (query.filter === "failed") {
          return task.status === "failed" || task.status === "interrupted";
        }
        if (query.filter === "completed") {
          return task.status === "completed";
        }
        return true;
      });
      return structuredClone(filtered);
    },
    async cancelProcessingTask(taskId) {
      const task = tasks.find((item) => item.id === taskId);
      if (!task) {
        throw new Error("任务不存在或已被移除");
      }
      task.status = "cancelled";
      task.availableActions = [];
      task.updatedAt = new Date().toISOString();
      return structuredClone(task);
    },
    async retryProcessingTask(taskId) {
      const task = tasks.find((item) => item.id === taskId);
      if (!task) {
        throw new Error("任务不存在或已被移除");
      }
      task.status = "queued";
      task.attempt += 1;
      task.error = null;
      task.availableActions = ["cancel"];
      task.updatedAt = new Date().toISOString();
      return structuredClone(task);
    },
    async reselectProcessingTask(taskId) {
      const task = tasks.find((item) => item.id === taskId);
      if (!task) {
        throw new Error("任务不存在或已被移除");
      }
      if (task.attempt >= task.maxAttempts) {
        throw new Error("任务已达到最大尝试次数");
      }
      task.artifactId = `artifact-reselected-${sequence++}`;
      task.status = "queued";
      task.attempt += 1;
      task.error = null;
      task.availableActions = ["cancel"];
      task.updatedAt = new Date().toISOString();
      return structuredClone(task);
    },
    async listMeetings(query) {
      const summary: MeetingSummary = {
        id: meeting.id,
        title: meeting.minutes.title,
        summary: meeting.minutes.summary,
        meetingStartAt: meeting.minutes.meetingTime.startAt,
        durationMs: meeting.durationMs,
        updatedAt: meeting.createdAt,
        templateName: meeting.templateName,
      };
      const normalizedQuery = query.trim().toLocaleLowerCase("zh-CN");
      const haystack = `${summary.title ?? ""} ${summary.summary ?? ""}`.toLocaleLowerCase("zh-CN");
      return normalizedQuery && !haystack.includes(normalizedQuery) ? [] : [structuredClone(summary)];
    },
    async getMeetingDetail(meetingId) {
      if (meetingId !== meeting.id) {
        throw new Error("会议记录不存在或已被删除");
      }
      return structuredClone(meeting);
    },
    async getMeetingMarkdownPreview(meetingId) {
      if (meetingId !== meeting.id) {
        throw new Error("无法预览：会议记录不存在");
      }
      return createMeetingMarkdown();
    },
    async exportMeetingMarkdown(meetingId) {
      if (meetingId !== meeting.id) {
        throw new Error("无法导出：会议记录不存在");
      }
      return { status: "exported", displayName: "产品交付节奏讨论.md" };
    },
    async getPublicSettings() {
      return structuredClone(settings);
    },
    async saveProviderSettings(input: SaveProviderSettingsInput) {
      settings = {
        transcription: {
          presetId: input.transcription.presetId,
          kind: input.transcription.kind,
          endpoint: input.transcription.endpoint,
          model: input.transcription.model,
          connectTimeoutMs: input.transcription.connectTimeoutMs,
          requestTimeoutMs: input.transcription.requestTimeoutMs,
          maxRetries: input.transcription.maxRetries,
          secretConfigured: resolveMockSecretStatus(
            settings.transcription.presetId,
            input.transcription.presetId,
            settings.transcription.secretConfigured,
            input.transcription.apiKey,
          ),
        },
        minutes: {
          presetId: input.minutes.presetId,
          kind: input.minutes.kind,
          endpoint: input.minutes.endpoint,
          model: input.minutes.model,
          connectTimeoutMs: input.minutes.connectTimeoutMs,
          requestTimeoutMs: input.minutes.requestTimeoutMs,
          maxRetries: input.minutes.maxRetries,
          secretConfigured: resolveMockSecretStatus(
            settings.minutes.presetId,
            input.minutes.presetId,
            settings.minutes.secretConfigured,
            input.minutes.apiKey,
          ),
        },
      };
      return structuredClone(settings);
    },
    async testProviderConnection(target) {
      const provider = settings[target];
      if (provider.kind === "mock") {
        return { ok: true, safeMessage: "Mock 服务可用" };
      }
      if (!provider.secretConfigured) {
        return { ok: false, safeMessage: "请先保存 API Key" };
      }
      return { ok: false, safeMessage: "真实 Provider 字段尚未完成最小验证，未发送网络请求" };
    },
  };
}
