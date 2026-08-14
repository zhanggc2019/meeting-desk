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
  processingDurationMs: 1_320_000,
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
    decisions: [{ content: "本周先完成本地全流程联调。", evidenceSegmentIds: ["segment-2"] }],
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
        mitigation: "先保持 Provider 抽象并使用本地测试流程联调",
        evidenceSegmentIds: ["segment-4"],
      },
    ],
  },
  transcript: {
    schemaVersion: "1",
    text: "我们先聚焦核心闭环。本地测试流程完成后，再接入真实服务。测试团队整理验收清单。真实服务字段仍需验证。",
    language: "zh-CN",
    durationMs: 2_430_000,
    segments: [
      { id: "segment-1", startMs: 0, endMs: 15_000, speakerLabel: "说话人 A", text: "我们先聚焦核心闭环。" },
      { id: "segment-2", startMs: 15_000, endMs: 34_000, speakerLabel: "说话人 B", text: "本地测试流程完成后，再接入真实服务。" },
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
    processingStartedAt: null,
    processingDurationMs: 0,
    sourceDurationMs: 2_430_000,
    estimatedProcessingMs: 3_600_000,
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
    processingStartedAt: null,
    processingDurationMs: 1_200_000,
    sourceDurationMs: 1_860_000,
    estimatedProcessingMs: 2_910_000,
    error: { code: "network_unavailable", safeMessage: "网络不可用，请检查连接后重试", retryable: true },
    availableActions: ["retry", "delete"],
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
    processingStartedAt: null,
    processingDurationMs: 1_320_000,
    sourceDurationMs: 2_430_000,
    estimatedProcessingMs: 1_320_000,
    error: null,
    availableActions: ["openMeeting", "delete"],
  },
];

const initialSettings: PublicSettings = {
  transcription: {
    presetId: "local_funasr",
    kind: "local_funasr",
    endpoint: "local://model/SenseVoiceSmall",
    model: "SenseVoiceSmall",
    localModelPath: "",
    secretConfigured: false,
    ready: true,
    readiness: "ready",
    validationMessage: "本地 SenseVoiceSmall 配置已就绪",
    connectTimeoutMs: 10_000,
    requestTimeoutMs: 3_600_000,
    maxRetries: 0,
  },
  minutes: {
    presetId: "deepseek",
    kind: "openai_compatible",
    endpoint: "https://api.deepseek.com/chat/completions",
    model: "deepseek-v4-flash",
    secretConfigured: false,
    ready: false,
    readiness: "incomplete",
    validationMessage: "请补充：API Key",
    connectTimeoutMs: 10_000,
    requestTimeoutMs: 120_000,
    maxRetries: 2,
  },
};

/** 生成与浏览器测试导出结果一致的 Markdown 预览文本。 */
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

我们先聚焦核心闭环。本地测试流程完成后，再接入真实服务。`;
}

/** 根据扩展名返回供浏览器测试校验使用的 MIME。 */
function inferMimeType(name: string, providedType: string): string | null {
  const extension = name.split(".").pop()?.toLowerCase();
  const mimeByExtension: Record<string, string> = {
    mp3: "audio/mpeg",
    wav: "audio/wav",
    m4a: "audio/mp4",
    mp4: "video/mp4",
    mov: "video/quicktime",
  };
  const inferred = extension ? (mimeByExtension[extension] ?? null) : null;
  if (!inferred) return null;
  return providedType === "" || providedType === inferred ? inferred : null;
}

/** 创建只含安全显示信息的浏览器测试导入候选项。 */
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
  currentEndpoint: string,
  nextEndpoint: string,
  currentConfigured: boolean,
  nextSecret: string | undefined,
): boolean {
  const sameBinding = currentPresetId === nextPresetId
    && (nextPresetId !== "custom_openai_compatible" || currentEndpoint.trim() === nextEndpoint.trim());
  return Boolean(nextSecret?.trim()) || (sameBinding && currentConfigured);
}

/** 判断浏览器测试客户端中的本地转写和在线纪要是否均已完成必要配置。 */
function areProvidersReady(settings: PublicSettings): boolean {
  return [settings.transcription, settings.minutes].every((provider) => (
    provider.kind !== "mock"
    && provider.endpoint.trim().length > 0
    && provider.model.trim().length > 0
    && (provider.kind === "local_funasr" || provider.secretConfigured)
  ));
}

/** 返回一个完全离线、确定性的浏览器测试客户端。 */
export function createMockDesktopClient(): DesktopClient {
  let tasks = initialTasks.map((task) => ({ ...task, availableActions: [...task.availableActions] }));
  let settings = structuredClone(initialSettings);
  let meetingDeleted = false;
  let sequence = 1;
  const artifactNames = new Map<string, string>();

  /** 按任务状态筛选浏览器测试数据。 */
  function filterTasks(query: TaskQuery): ProcessingTask[] {
    return tasks.filter((task) => {
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
  }

  /** 按关键词筛选浏览器测试会议摘要。 */
  function filterMeetings(query: string): MeetingSummary[] {
    if (meetingDeleted) return [];
    const summary: MeetingSummary = {
      id: meeting.id,
      title: meeting.minutes.title,
      summary: meeting.minutes.summary,
      meetingStartAt: meeting.minutes.meetingTime.startAt,
      durationMs: meeting.durationMs,
      processingDurationMs: meeting.processingDurationMs,
      updatedAt: meeting.createdAt,
      templateName: meeting.templateName,
    };
    const normalizedQuery = query.trim().toLocaleLowerCase("zh-CN");
    const haystack = `${summary.title ?? ""} ${summary.summary ?? ""}`.toLocaleLowerCase("zh-CN");
    return normalizedQuery && !haystack.includes(normalizedQuery) ? [] : [summary];
  }

  /** 将测试数据转换为与桌面端一致的分页结果。 */
  function paginate<T>(items: T[], page: number, pageSize: number) {
    const totalPages = Math.max(1, Math.ceil(items.length / pageSize));
    const normalizedPage = Math.min(Math.max(1, page), totalPages);
    return {
      items: structuredClone(items.slice((normalizedPage - 1) * pageSize, normalizedPage * pageSize)),
      total: items.length,
      page: normalizedPage,
      pageSize,
      totalPages,
    };
  }

  return {
    async selectAudioFiles(mode) {
      if (!areProvidersReady(settings)) {
        throw new Error("请先完成语音转写和会议纪要服务配置，再选择音频或视频");
      }
      const sampleFiles =
        mode === "batch"
          ? [
              { name: "演示会议.mp3", size: 12_582_912, type: "audio/mpeg" },
              { name: "课程录像.mp4", size: 18_388_608, type: "video/mp4" },
            ]
          : [{ name: "演示会议.mp3", size: 12_582_912, type: "audio/mpeg" }];
      const candidates = sampleFiles.map((file, index) => createCandidate(file, sequence + index));
      sequence += sampleFiles.length;
      candidates.forEach((candidate) => {
        if (candidate.artifactId) artifactNames.set(candidate.artifactId, candidate.displayName);
      });
      return candidates;
    },
    async registerBrowserFiles(files) {
      if (!areProvidersReady(settings)) {
        throw new Error("请先完成语音转写和会议纪要服务配置，再选择音频或视频");
      }
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
        processingStartedAt: null,
        processingDurationMs: 0,
        sourceDurationMs: 1_860_000,
        estimatedProcessingMs: 2_910_000,
        error: null,
        availableActions: ["cancel" as const],
      }));
      tasks = [...created, ...tasks];
      return structuredClone(created);
    },
    async listProcessingTasks(query: TaskQuery) {
      return structuredClone(filterTasks(query));
    },
    async listProcessingTasksPage(query) {
      return paginate(filterTasks(query), query.page, query.pageSize);
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
    async deleteProcessingTask(taskId) {
      const task = tasks.find((item) => item.id === taskId);
      if (!task) return false;
      if (!task.availableActions.includes("delete")) {
        throw new Error("该任务当前不能删除");
      }
      tasks = tasks.filter((item) => item.id !== taskId);
      artifactNames.delete(task.artifactId);
      if (task.meetingId === meeting.id) meetingDeleted = true;
      return true;
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
      return structuredClone(filterMeetings(query));
    },
    async listMeetingsPage(query) {
      return paginate(filterMeetings(query.query), query.page, query.pageSize);
    },
    async getMeetingDetail(meetingId) {
      if (meetingDeleted || meetingId !== meeting.id) {
        throw new Error("会议记录不存在或已被删除");
      }
      return structuredClone(meeting);
    },
    async getMeetingMarkdownPreview(meetingId) {
      if (meetingDeleted || meetingId !== meeting.id) {
        throw new Error("无法预览：会议记录不存在");
      }
      return createMeetingMarkdown();
    },
    async deleteMeeting(meetingId) {
      if (meetingDeleted || meetingId !== meeting.id) return false;
      meetingDeleted = true;
      tasks = tasks.filter((task) => task.meetingId !== meetingId);
      return true;
    },
    async exportMeetingMarkdown(meetingId) {
      if (meetingDeleted || meetingId !== meeting.id) {
        throw new Error("无法导出：会议记录不存在");
      }
      return { status: "exported", displayName: "产品交付节奏讨论.md" };
    },
    async getPublicSettings() {
      return structuredClone(settings);
    },
    async selectLocalModelDirectory() {
      return "D:\\Models\\SenseVoiceSmall";
    },
    async saveProviderSettings(input: SaveProviderSettingsInput) {
      const transcriptionSecretConfigured = resolveMockSecretStatus(
        settings.transcription.presetId,
        input.transcription.presetId,
        settings.transcription.endpoint,
        input.transcription.endpoint,
        settings.transcription.secretConfigured,
        input.transcription.apiKey,
      );
      const minutesSecretConfigured = resolveMockSecretStatus(
        settings.minutes.presetId,
        input.minutes.presetId,
        settings.minutes.endpoint,
        input.minutes.endpoint,
        settings.minutes.secretConfigured,
        input.minutes.apiKey,
      );
      const localTranscription = input.transcription.kind === "local_funasr";
      settings = {
        transcription: {
          presetId: input.transcription.presetId,
          kind: input.transcription.kind,
          endpoint: input.transcription.endpoint,
          model: input.transcription.model,
          localModelPath: input.transcription.localModelPath ?? "",
          connectTimeoutMs: input.transcription.connectTimeoutMs,
          requestTimeoutMs: input.transcription.requestTimeoutMs,
          maxRetries: input.transcription.maxRetries,
          secretConfigured: transcriptionSecretConfigured,
          ready: localTranscription || transcriptionSecretConfigured,
          readiness: localTranscription || transcriptionSecretConfigured ? "ready" : "incomplete",
          validationMessage: localTranscription
            ? "本地 SenseVoiceSmall 配置已就绪"
            : (transcriptionSecretConfigured ? "真实 Provider 配置已就绪" : "请补充：API Key"),
        },
        minutes: {
          presetId: input.minutes.presetId,
          kind: input.minutes.kind,
          endpoint: input.minutes.endpoint,
          model: input.minutes.model,
          localModelPath: "",
          connectTimeoutMs: input.minutes.connectTimeoutMs,
          requestTimeoutMs: input.minutes.requestTimeoutMs,
          maxRetries: input.minutes.maxRetries,
          secretConfigured: minutesSecretConfigured,
          ready: minutesSecretConfigured,
          readiness: minutesSecretConfigured ? "ready" : "incomplete",
          validationMessage: minutesSecretConfigured ? "真实 Provider 配置已就绪" : "请补充：API Key",
        },
      };
      return structuredClone(settings);
    },
    async testProviderConnection(target, input) {
      const provider = settings[target];
      if (provider.kind === "local_funasr") {
        return { ok: false, safeMessage: "浏览器测试环境无法检查本地模型，请在 Windows 桌面应用中检查环境" };
      }
      if (!provider.secretConfigured && !input?.apiKey?.trim()) {
        return { ok: false, safeMessage: "请先保存 API Key" };
      }
      return { ok: false, safeMessage: "浏览器测试环境不发送网络请求，请在 Windows 桌面应用中测试连接" };
    },
  };
}
