import { invoke } from "@tauri-apps/api/core";
import type {
  BrowserFileDescriptor,
  ExportResult,
  ImportMode,
  ImportCandidate,
  MeetingDetail,
  MeetingSummary,
  MinutesTemplate,
  ProcessingTask,
  PublicSettings,
  SaveProviderSettingsInput,
  TaskQuery,
} from "../contracts/desktop";

export const DESKTOP_COMMANDS = {
  selectAudioFiles: "select_audio_files",
  releaseAudioArtifact: "release_audio_artifact",
  createProcessingTasks: "create_processing_tasks",
  listProcessingTasks: "list_processing_tasks",
  cancelProcessingTask: "cancel_processing_task",
  retryProcessingTask: "retry_processing_task",
  deleteProcessingTask: "delete_processing_task",
  reselectProcessingTask: "reselect_processing_task",
  listMeetings: "list_meetings",
  getMeetingDetail: "get_meeting_detail",
  getMeetingMarkdownPreview: "get_meeting_markdown_preview",
  deleteMeeting: "delete_meeting",
  exportMeetingMarkdown: "export_meeting_markdown",
  listMinutesTemplates: "list_minutes_templates",
  getPublicSettings: "get_public_settings",
  selectLocalModelDirectory: "select_local_model_directory",
  saveProviderSettings: "save_provider_settings",
  testProviderConnection: "test_provider_connection",
} as const;

export interface DesktopClient {
  /** 打开桌面文件对话框并返回经过后端校验的候选项。 */
  selectAudioFiles(mode: ImportMode): Promise<ImportCandidate[]>;
  /** 为浏览器界面测试注册文件元数据。 */
  registerBrowserFiles(files: BrowserFileDescriptor[]): Promise<ImportCandidate[]>;
  /** 删除尚未提交的受管暂存副本。 */
  releaseAudioArtifact(artifactId: string): Promise<void>;
  /** 返回后端已注册的纪要模板。 */
  listMinutesTemplates(): Promise<MinutesTemplate[]>;
  /** 为每个 artifact 创建独立处理任务。 */
  createProcessingTasks(artifactIds: string[], templateId: string): Promise<ProcessingTask[]>;
  /** 按持久化状态读取处理任务。 */
  listProcessingTasks(query: TaskQuery): Promise<ProcessingTask[]>;
  /** 请求取消单个处理任务。 */
  cancelProcessingTask(taskId: string): Promise<ProcessingTask>;
  /** 请求重试后端明确允许重试的任务。 */
  retryProcessingTask(taskId: string): Promise<ProcessingTask>;
  /** 删除失败任务及不再使用的受管临时文件。 */
  deleteProcessingTask(taskId: string): Promise<boolean>;
  /** 通过系统文件对话框重新选择音频并续接中断任务。 */
  reselectProcessingTask(taskId: string): Promise<ProcessingTask>;
  /** 仅在本地会议仓库中搜索会议。 */
  listMeetings(query: string): Promise<MeetingSummary[]>;
  /** 按 ID 读取已经持久化的纪要和逐字稿。 */
  getMeetingDetail(meetingId: string): Promise<MeetingDetail>;
  /** 返回与导出文件一致的 Markdown 文本，仅用于本地预览。 */
  getMeetingMarkdownPreview(meetingId: string): Promise<string>;
  /** 删除本地会议、逐字稿、纪要及关联任务，不接触用户原始文件。 */
  deleteMeeting(meetingId: string): Promise<boolean>;
  /** 使用桌面保存对话框导出 Markdown。 */
  exportMeetingMarkdown(meetingId: string): Promise<ExportResult>;
  /** 读取绝不包含密钥值的公开设置。 */
  getPublicSettings(): Promise<PublicSettings>;
  /** 选择并校验一个本机 SenseVoiceSmall 模型目录。 */
  selectLocalModelDirectory(): Promise<string | null>;
  /** 保存配置并只返回公开配置状态。 */
  saveProviderSettings(input: SaveProviderSettingsInput): Promise<PublicSettings>;
  /** 返回不包含请求或响应正文的连接测试结果。 */
  testProviderConnection(target: "transcription" | "minutes"): Promise<{ ok: boolean; safeMessage: string }>;
}

/** 判断当前页面是否运行在 Tauri WebView 中。 */
export function isTauriRuntime(): boolean {
  const runtimeWindow = window as Window & { __TAURI_INTERNALS__?: unknown };
  return runtimeWindow.__TAURI_INTERNALS__ !== undefined;
}

/** 把未知错误转换为不包含远程响应或请求内容的安全提示。 */
export function getSafeErrorMessage(error: unknown): string {
  if (typeof error === "object" && error !== null && "safeMessage" in error) {
    const safeMessage = (error as { safeMessage?: unknown }).safeMessage;
    if (typeof safeMessage === "string" && safeMessage.trim().length > 0) {
      return safeMessage;
    }
  }
  return "操作未完成，请稍后重试";
}

/** 创建仅在 Tauri 桌面端使用的 invoke 适配器。 */
export function createTauriDesktopClient(): DesktopClient {
  return {
    async selectAudioFiles(mode) {
      return invoke<ImportCandidate[]>(DESKTOP_COMMANDS.selectAudioFiles, { selectionMode: mode });
    },
    async registerBrowserFiles() {
      throw new Error("桌面端请使用系统文件选择器");
    },
    async releaseAudioArtifact(artifactId) {
      await invoke<boolean>(DESKTOP_COMMANDS.releaseAudioArtifact, { artifactId });
    },
    async listMinutesTemplates() {
      return invoke<MinutesTemplate[]>(DESKTOP_COMMANDS.listMinutesTemplates);
    },
    async createProcessingTasks(artifactIds, templateId) {
      return invoke<ProcessingTask[]>(DESKTOP_COMMANDS.createProcessingTasks, {
        artifactIds,
        templateId,
      });
    },
    async listProcessingTasks(query) {
      return invoke<ProcessingTask[]>(DESKTOP_COMMANDS.listProcessingTasks, { query });
    },
    async cancelProcessingTask(taskId) {
      return invoke<ProcessingTask>(DESKTOP_COMMANDS.cancelProcessingTask, { taskId });
    },
    async retryProcessingTask(taskId) {
      return invoke<ProcessingTask>(DESKTOP_COMMANDS.retryProcessingTask, { taskId });
    },
    async deleteProcessingTask(taskId) {
      return invoke<boolean>(DESKTOP_COMMANDS.deleteProcessingTask, { taskId });
    },
    async reselectProcessingTask(taskId) {
      return invoke<ProcessingTask>(DESKTOP_COMMANDS.reselectProcessingTask, { taskId });
    },
    async listMeetings(query) {
      return invoke<MeetingSummary[]>(DESKTOP_COMMANDS.listMeetings, { query });
    },
    async getMeetingDetail(meetingId) {
      return invoke<MeetingDetail>(DESKTOP_COMMANDS.getMeetingDetail, { meetingId });
    },
    async getMeetingMarkdownPreview(meetingId) {
      return invoke<string>(DESKTOP_COMMANDS.getMeetingMarkdownPreview, { meetingId });
    },
    async deleteMeeting(meetingId) {
      return invoke<boolean>(DESKTOP_COMMANDS.deleteMeeting, { meetingId });
    },
    async exportMeetingMarkdown(meetingId) {
      return invoke<ExportResult>(DESKTOP_COMMANDS.exportMeetingMarkdown, { meetingId });
    },
    async getPublicSettings() {
      return invoke<PublicSettings>(DESKTOP_COMMANDS.getPublicSettings);
    },
    async selectLocalModelDirectory() {
      return invoke<string | null>(DESKTOP_COMMANDS.selectLocalModelDirectory);
    },
    async saveProviderSettings(input) {
      return invoke<PublicSettings>(DESKTOP_COMMANDS.saveProviderSettings, { input });
    },
    async testProviderConnection(target) {
      return invoke<{ ok: boolean; safeMessage: string }>(DESKTOP_COMMANDS.testProviderConnection, { target });
    },
  };
}
