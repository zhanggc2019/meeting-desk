export type AppPage = "workspace" | "tasks" | "meetings" | "meeting-detail";

export type ValidationStatus = "validating" | "ready" | "invalid";

export type ImportMode = "single" | "batch";

export interface ImportCandidate {
  id: string;
  artifactId: string | null;
  displayName: string;
  mimeType: string | null;
  sizeBytes: number | null;
  durationMs: number | null;
  validationStatus: ValidationStatus;
  safeMessage: string | null;
}

export interface BrowserFileDescriptor {
  name: string;
  size: number;
  type: string;
}

export interface MinutesTemplate {
  id: string;
  version: string;
  name: string;
  description: string;
}

export type TaskStatus =
  | "queued"
  | "preparing"
  | "uploading"
  | "transcribing"
  | "validating_transcript"
  | "summarizing"
  | "validating_minutes"
  | "saving"
  | "retry_wait"
  | "cancel_requested"
  | "interrupted"
  | "completed"
  | "failed"
  | "cancelled";

export type TaskAction = "cancel" | "retry" | "openMeeting" | "reselectFile";

export interface TaskError {
  code: string;
  safeMessage: string;
  retryable: boolean;
  httpStatus?: number;
  retryAfterMs?: number;
}

export interface ProcessingTask {
  id: string;
  artifactId: string;
  batchId: string | null;
  displayName: string;
  meetingId: string | null;
  templateId: string;
  status: TaskStatus;
  attempt: number;
  maxAttempts: number;
  progress: number | null;
  updatedAt: string;
  createdAt: string;
  error: TaskError | null;
  availableActions: TaskAction[];
}

export interface TaskQuery {
  filter: "all" | "active" | "failed" | "completed";
}

export interface MeetingSummary {
  id: string;
  title: string | null;
  summary: string | null;
  meetingStartAt: string | null;
  durationMs: number | null;
  processingDurationMs: number | null;
  updatedAt: string;
  templateName: string;
}

export interface TranscriptSegment {
  id: string;
  startMs?: number;
  endMs?: number;
  speakerLabel?: string;
  text: string;
  confidence?: number;
}

export interface Transcript {
  schemaVersion: "1";
  text: string;
  language?: string;
  durationMs?: number;
  segments: TranscriptSegment[];
}

export interface SupportedStatement {
  content: string;
  evidenceSegmentIds: string[];
}

export interface Topic {
  title: string;
  summary: string | null;
  evidenceSegmentIds: string[];
}

export interface ActionItem {
  description: string;
  owner: string | null;
  dueDateText: string | null;
  dueDate: string | null;
  evidenceSegmentIds: string[];
}

export interface RiskOrIssue {
  kind: "risk" | "issue";
  description: string;
  impact: string | null;
  mitigation: string | null;
  evidenceSegmentIds: string[];
}

export interface MeetingMinutes {
  schemaVersion: "1.0.0";
  title: string | null;
  titleSource: "context" | "generated" | "unknown";
  meetingTime: {
    startAt: string | null;
    endAt: string | null;
  };
  participants: string[];
  summary: string | null;
  topics: Topic[];
  conclusions: SupportedStatement[];
  decisions: SupportedStatement[];
  actionItems: ActionItem[];
  risksAndIssues: RiskOrIssue[];
}

export interface MeetingDetail {
  id: string;
  templateName: string;
  durationMs: number | null;
  processingDurationMs: number | null;
  createdAt: string;
  minutes: MeetingMinutes;
  transcript: Transcript;
}

export type ProviderKind =
  | "mock"
  | "dashscope_funasr"
  | "xiaomi_mimo"
  | "volcengine_asr"
  | "openai_compatible";
export type ProviderReadiness = "incomplete" | "mockExperience" | "ready";
export type ProviderPresetId =
  | "mock"
  | "dashscope_funasr_cn"
  | "dashscope_funasr_intl"
  | "xiaomi_mimo_asr"
  | "volcengine_asr_flash"
  | "xiaomi_mimo_llm"
  | "deepseek"
  | "aliyun_bailian"
  | "custom_openai_compatible";

export interface PublicProviderSettings {
  /** 后端解析后的受信任预设；可选以兼容尚未返回该字段的旧桌面壳。 */
  presetId?: ProviderPresetId;
  kind: ProviderKind;
  endpoint: string;
  model: string;
  secretConfigured: boolean;
  /** 后端可选提供的完整可用状态；旧版本客户端会使用公开字段派生。 */
  ready?: boolean;
  /** 后端计算的配置状态；可选以兼容旧版桌面壳。 */
  readiness?: ProviderReadiness;
  /** 不包含密钥或响应正文的配置校验提示。 */
  validationMessage?: string | null;
  connectTimeoutMs: number;
  requestTimeoutMs: number;
  maxRetries: number;
}

export interface PublicSettings {
  transcription: PublicProviderSettings;
  minutes: PublicProviderSettings;
}

export interface ProviderSettingsInput {
  presetId: ProviderPresetId;
  kind: ProviderKind;
  endpoint: string;
  model: string;
  apiKey?: string;
  connectTimeoutMs: number;
  requestTimeoutMs: number;
  maxRetries: number;
}

export interface SaveProviderSettingsInput {
  transcription: ProviderSettingsInput;
  minutes: ProviderSettingsInput;
}

export interface ExportResult {
  status: "exported" | "cancelled";
  displayName?: string;
}
