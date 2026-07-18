import { FileAudio, FileVideo2, FolderOpen, Plus, Settings2, Trash2, UploadCloud, X } from "lucide-react";
import { useEffect, useMemo, useState, type ChangeEvent, type DragEvent } from "react";
import type { ImportCandidate, ImportMode, MinutesTemplate, ProcessingTask, PublicProviderSettings, PublicSettings } from "../../contracts/desktop";
import { useDesktopClient } from "../../services/DesktopClientContext";
import { getSafeErrorMessage } from "../../services/desktopClient";
import { useAppStore } from "../../stores/appStore";
import { formatBytes, formatDuration, formatRelativeDate, getTaskStatusLabel } from "../../utils/format";

/** 合并新候选项并按候选 ID 去重。 */
function mergeCandidates(current: ImportCandidate[], incoming: ImportCandidate[]): ImportCandidate[] {
  const byId = new Map(current.map((candidate) => [candidate.id, candidate]));
  incoming.forEach((candidate) => byId.set(candidate.id, candidate));
  return Array.from(byId.values());
}

/** 根据公开状态判断真实 Provider 的必要配置是否已填写。 */
function hasCompleteRealProviderConfig(provider: PublicProviderSettings): boolean {
  if (provider.kind === "mock") return false;
  if (provider.readiness !== undefined) return provider.readiness === "ready";
  return provider.ready ?? (
    provider.endpoint.trim().length > 0
    && provider.model.trim().length > 0
    && provider.secretConfigured
  );
}

/** 返回不暴露敏感配置的 Provider 状态文案。 */
function getProviderStatusCopy(provider: PublicProviderSettings): string {
  if (hasCompleteRealProviderConfig(provider)) return "真实服务配置已填写";
  if (provider.kind === "mock") return "旧版演示配置已停用，请重新配置";
  return provider.validationMessage?.trim() || "配置不完整，请检查地址、模型和密钥";
}

/** 判断安全显示元数据是否表示视频文件。 */
function isVideoFile(name: string, mimeType?: string | null): boolean {
  return mimeType?.startsWith("video/") === true || /\.(mp4|mov)$/i.test(name);
}

/** 渲染单文件和批量文件共用的离线媒体工作台。 */
export function WorkspacePage() {
  const client = useDesktopClient();
  const navigate = useAppStore((state) => state.navigate);
  const openSettings = useAppStore((state) => state.openSettings);
  const setTaskAttentionCount = useAppStore((state) => state.setTaskAttentionCount);
  const settingsRevision = useAppStore((state) => state.settingsRevision);
  const [candidates, setCandidates] = useState<ImportCandidate[]>([]);
  const [importMode, setImportMode] = useState<ImportMode>("single");
  const [templates, setTemplates] = useState<MinutesTemplate[]>([]);
  const [selectedTemplate, setSelectedTemplate] = useState("adaptive");
  const [recentTasks, setRecentTasks] = useState<ProcessingTask[]>([]);
  const [settings, setSettings] = useState<PublicSettings | null>(null);
  const [isDragging, setIsDragging] = useState(false);
  const [isSelecting, setIsSelecting] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const readyCandidates = useMemo(
    () => candidates.filter((candidate) => candidate.validationStatus === "ready" && candidate.artifactId),
    [candidates],
  );

  const invalidCandidateCount = useMemo(
    () => candidates.filter((candidate) => candidate.validationStatus === "invalid").length,
    [candidates],
  );

  const totalCandidateSize = useMemo(
    () => candidates.reduce((total, candidate) => total + (candidate.sizeBytes ?? 0), 0),
    [candidates],
  );

  const selectedTemplateDetail = useMemo(
    () => templates.find((template) => template.id === selectedTemplate) ?? null,
    [selectedTemplate, templates],
  );

  const providerSetup = useMemo(() => {
    if (!settings) return { complete: false };
    return { complete: hasCompleteRealProviderConfig(settings.transcription) && hasCompleteRealProviderConfig(settings.minutes) };
  }, [settings]);

  const canProcess = providerSetup.complete;
  const canSelectMedia = settings !== null && providerSetup.complete;

  useEffect(() => {
    let active = true;
    Promise.all([client.listMinutesTemplates(), client.listProcessingTasks({ filter: "all" }), client.getPublicSettings()])
      .then(([availableTemplates, tasks, publicSettings]) => {
        if (!active) return;
        setTemplates(availableTemplates);
        setSelectedTemplate((current) => availableTemplates.some((template) => template.id === current)
          ? current
          : (availableTemplates[0]?.id ?? "standard_meeting"));
        setRecentTasks(tasks.slice(0, 3));
        setSettings(publicSettings);
        setTaskAttentionCount(tasks.filter((task) => ["failed", "interrupted"].includes(task.status)).length);
      })
      .catch((reason: unknown) => active && setError(getSafeErrorMessage(reason)));

    return () => {
      active = false;
    };
  }, [client, setTaskAttentionCount, settingsRevision]);

  /** 将新候选项加入当前模式，并防止旧版桌面壳向单文件模式返回多个文件。 */
  async function appendCandidates(selected: ImportCandidate[]) {
    if (importMode === "single" && (selected.length > 1 || candidates.length > 0)) {
      await Promise.all(selected.flatMap((candidate) => candidate.artifactId
        ? [client.releaseAudioArtifact(candidate.artifactId)]
        : []));
      setError("单个文件模式一次只能保留 1 个文件，请清空列表后重试或切换到批量处理");
      return;
    }
    setCandidates((current) => mergeCandidates(current, selected));
  }

  /** 使用桌面服务按当前导入模式打开系统文件选择器。 */
  async function handleSelectFiles() {
    if (!canSelectMedia) {
      setError("请先完成语音转写和会议纪要服务配置，再选择音频或视频");
      return;
    }
    setIsSelecting(true);
    setError(null);
    try {
      const selected = await client.selectAudioFiles(importMode);
      await appendCandidates(selected);
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setIsSelecting(false);
    }
  }

  /** 在浏览器测试环境中校验通过 input 选择的文件。 */
  async function handleBrowserFiles(event: ChangeEvent<HTMLInputElement>) {
    if (!canSelectMedia) {
      event.target.value = "";
      setError("请先完成语音转写和会议纪要服务配置，再选择音频或视频");
      return;
    }
    const files = Array.from(event.target.files ?? []);
    if (files.length === 0) return;
    setError(null);
    try {
      const selected = await client.registerBrowserFiles(files.map((file) => ({ name: file.name, size: file.size, type: file.type })));
      await appendCandidates(selected);
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      event.target.value = "";
    }
  }

  /** 处理浏览器测试中的文件拖放；桌面端只使用系统文件选择器。 */
  async function handleDrop(event: DragEvent<HTMLDivElement>) {
    event.preventDefault();
    setIsDragging(false);
    if (!canSelectMedia) {
      setError("请先完成语音转写和会议纪要服务配置，再选择音频或视频");
      return;
    }
    const files = Array.from(event.dataTransfer.files);
    if (files.length === 0) {
      setError("未检测到可处理的音频或视频文件");
      return;
    }
    if (importMode === "single" && (files.length > 1 || candidates.length > 0)) {
      setError("单个文件模式一次只能添加 1 个文件，请清空列表后重试或切换到批量处理");
      return;
    }
    try {
      const selected = await client.registerBrowserFiles(files.map((file) => ({ name: file.name, size: file.size, type: file.type })));
      await appendCandidates(selected);
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    }
  }

  /** 清理受管暂存副本后，从尚未提交的审阅列表移除候选项。 */
  async function removeCandidate(candidateId: string) {
    const candidate = candidates.find((item) => item.id === candidateId);
    if (candidate?.artifactId) {
      try {
        await client.releaseAudioArtifact(candidate.artifactId);
      } catch (reason) {
        setError(getSafeErrorMessage(reason));
        return;
      }
    }
    setCandidates((current) => current.filter((candidate) => candidate.id !== candidateId));
  }

  /** 清理全部未提交的受管暂存副本并清空复核列表。 */
  async function clearCandidates() {
    try {
      await Promise.all(
        candidates.flatMap((candidate) => candidate.artifactId
          ? [client.releaseAudioArtifact(candidate.artifactId)]
          : []),
      );
      setCandidates([]);
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    }
  }

  /** 为全部已通过校验的文件创建独立处理任务。 */
  async function submitCandidates() {
    const artifactIds = readyCandidates.flatMap((candidate) => candidate.artifactId ? [candidate.artifactId] : []);
    if (artifactIds.length === 0 || isSubmitting) return;
    if (!canProcess) {
      setError("请先完成语音转写和会议纪要服务配置");
      return;
    }
    setIsSubmitting(true);
    setError(null);
    try {
      await client.createProcessingTasks(artifactIds, selectedTemplate);
      setCandidates((current) => current.filter((candidate) => !artifactIds.includes(candidate.artifactId ?? "")));
      navigate("tasks");
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setIsSubmitting(false);
    }
  }

  return (
    <div className="page workspace-page">
      <header className="page-header">
        <div>
          <span className="eyebrow">离线媒体</span>
          <h1 tabIndex={-1}>转写工作台</h1>
          <p>选择一个或多个本地音频或视频文件，生成逐字稿和结构化会议纪要。</p>
        </div>
        <button className="button secondary" type="button" onClick={() => navigate("tasks")}>查看任务队列</button>
      </header>

      {!providerSetup.complete && settings ? (
        <section className="setup-guide" aria-labelledby="setup-title">
          <div className="setup-guide-copy">
            <span className="eyebrow">首次使用设置</span>
            <h2 id="setup-title">开始前，请先连接两项服务</h2>
            <p>完成语音转写和纪要生成服务配置后，才可以选择本地媒体；实际连通性以连接测试结果为准。</p>
          </div>
          <div className="setup-checklist" aria-label="服务配置状态">
            <div>
              <span className="setup-index">01</span>
              <span><strong>FunASR / ASR 转写接口</strong><small>{getProviderStatusCopy(settings.transcription)}</small></span>
            </div>
            <div>
              <span className="setup-index">02</span>
              <span><strong>纪要生成大模型接口</strong><small>{getProviderStatusCopy(settings.minutes)}</small></span>
            </div>
          </div>
          <div className="setup-actions">
            <button className="button primary" type="button" onClick={openSettings}><Settings2 size={16} aria-hidden="true" />打开服务设置</button>
          </div>
        </section>
      ) : null}

      {providerSetup.complete ? <div className="provider-ready-strip" role="status"><span aria-hidden="true" />两项真实服务配置已填写（实际连通性以测试结果为准）</div> : null}

      <section className="import-mode-section" aria-labelledby="import-mode-title">
        <div className="import-mode-heading">
          <div>
            <span className="eyebrow">处理方式</span>
            <h2 id="import-mode-title">选择导入模式</h2>
          </div>
          {candidates.length > 0 ? <small>清空当前列表后可切换模式</small> : null}
        </div>
        <div className="import-mode-switch" role="group" aria-label="导入模式">
          <button
            className={`import-mode-option${importMode === "single" ? " is-active" : ""}`}
            type="button"
            aria-label="单个文件"
            aria-pressed={importMode === "single"}
            disabled={candidates.length > 0}
            onClick={() => setImportMode("single")}
          >
            <strong>单个文件</strong>
            <small>一次处理 1 个录音或视频</small>
          </button>
          <button
            className={`import-mode-option${importMode === "batch" ? " is-active" : ""}`}
            type="button"
            aria-label="批量处理"
            aria-pressed={importMode === "batch"}
            disabled={candidates.length > 0}
            onClick={() => setImportMode("batch")}
          >
            <strong>批量处理</strong>
            <small>一次导入多个媒体文件</small>
          </button>
        </div>
        {importMode === "batch" ? (
          <p className="batch-mode-note">每个媒体文件会创建独立任务；单个文件失败不会影响本批次的其他文件。</p>
        ) : null}
      </section>

      <section aria-labelledby="import-title">
        <div
          className={`file-dropzone${isDragging ? " is-dragging" : ""}${canSelectMedia ? "" : " is-disabled"}`}
          aria-labelledby="import-title"
          aria-disabled={!canSelectMedia}
          onDragEnter={(event) => { event.preventDefault(); if (canSelectMedia) setIsDragging(true); }}
          onDragOver={(event) => event.preventDefault()}
          onDragLeave={() => setIsDragging(false)}
          onDrop={handleDrop}
        >
          <div className="dropzone-icon" aria-hidden="true"><UploadCloud size={28} strokeWidth={1.6} /></div>
          <h2 id="import-title">{!canSelectMedia ? "配置服务后选择媒体" : importMode === "batch" ? "批量添加离线媒体" : "选择一个离线媒体文件"}</h2>
          <p>{!canSelectMedia
            ? "语音转写和会议纪要服务均配置完成后，此处会自动启用。"
            : importMode === "batch"
            ? "可一次选择多个文件，也可分多次继续添加；源文件只读。"
            : candidates.length > 0
              ? "当前文件已加入列表，移除或清空后可重新选择。"
              : "一次选择一个文件，源文件只读，不会被移动或修改。"}</p>
          <div className="dropzone-actions">
            <button className="button primary" type="button" onClick={(event) => { event.stopPropagation(); void handleSelectFiles(); }} disabled={!canSelectMedia || isSelecting || (importMode === "single" && candidates.length > 0)}>
              <FolderOpen size={17} aria-hidden="true" />
              {isSelecting ? "正在打开" : importMode === "batch" ? "批量选择媒体" : "选择音频或视频"}
            </button>
          </div>
          <input className="visually-hidden" aria-label={importMode === "batch" ? "批量选择本地媒体文件" : "选择本地媒体文件"} type="file" accept=".mp3,.wav,.m4a,.mp4,.mov,audio/mpeg,audio/wav,audio/mp4,video/mp4,video/quicktime" multiple={importMode === "batch"} disabled={!canSelectMedia} onChange={handleBrowserFiles} />
          <small>支持 WAV、MP3、M4A、MP4、MOV；视频需包含 AAC 或 ALAC 音轨</small>
        </div>
      </section>

      {error ? <div className="inline-alert error" role="alert"><span>{error}</span><button type="button" onClick={() => setError(null)}>关闭</button></div> : null}

      {candidates.length > 0 ? (
        <section className="review-section" aria-labelledby="review-title">
          <div className="section-heading">
            <div>
              <h2 id="review-title">{importMode === "batch" ? "本批次文件" : "待处理文件"}</h2>
              {importMode === "batch" ? (
                <p className="batch-summary">
                  <span>{candidates.length} 个文件</span>
                  <span>{readyCandidates.length} 个可处理</span>
                  <span>{invalidCandidateCount} 个校验失败</span>
                  <span>合计 {formatBytes(totalCandidateSize)}</span>
                </p>
              ) : <p>已选择 1 个文件 · {readyCandidates.length} 个可处理</p>}
            </div>
            {importMode === "batch" ? <button className="button quiet" type="button" disabled={!canSelectMedia} onClick={() => void handleSelectFiles()}><Plus size={16} aria-hidden="true" />继续添加</button> : null}
          </div>
          <div className="table-wrap">
            <table>
              <thead><tr><th>文件名</th><th>大小</th><th>时长</th><th>校验状态</th><th><span className="visually-hidden">操作</span></th></tr></thead>
              <tbody>
                {candidates.map((candidate) => (
                  <tr key={candidate.id}>
                    <td><span className="file-name">{isVideoFile(candidate.displayName, candidate.mimeType) ? <FileVideo2 size={16} aria-hidden="true" /> : <FileAudio size={16} aria-hidden="true" />}{candidate.displayName}</span></td>
                    <td>{formatBytes(candidate.sizeBytes)}</td>
                    <td>{formatDuration(candidate.durationMs)}</td>
                    <td><span className={`validation ${candidate.validationStatus}`}>{candidate.safeMessage ?? (candidate.validationStatus === "ready" ? "可处理" : "正在校验")}</span></td>
                    <td className="cell-actions"><button className="icon-button" type="button" aria-label={`移除 ${candidate.displayName}`} onClick={() => void removeCandidate(candidate.id)}><X size={16} /></button></td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div className="review-footer">
            <label className="field compact-field">会议纪要模板
              <select value={selectedTemplate} onChange={(event) => setSelectedTemplate(event.target.value)}>
                {templates.map((template) => <option key={template.id} value={template.id}>{template.name}</option>)}
              </select>
              {selectedTemplateDetail ? <small className="field-help">{selectedTemplateDetail.description}</small> : null}
            </label>
            <div className="footer-actions">
              <button className="button quiet" type="button" onClick={() => void clearCandidates()}><Trash2 size={16} aria-hidden="true" />清空列表</button>
              <button className="button primary" type="button" disabled={readyCandidates.length === 0 || isSubmitting || !canProcess} onClick={() => void submitCandidates()}>
                {isSubmitting
                  ? "正在创建任务"
                  : !canProcess
                    ? "请先配置服务"
                    : importMode === "batch"
                      ? `创建 ${readyCandidates.length} 个处理任务`
                      : "开始处理"}
              </button>
            </div>
          </div>
        </section>
      ) : null}

      <section className="recent-section" aria-labelledby="recent-title">
        <div className="section-heading"><div><h2 id="recent-title">最近任务</h2><p>任务状态来自本地持久化记录。</p></div></div>
        {recentTasks.length === 0 ? <div className="empty-inline">当前没有处理任务</div> : (
          <div className="compact-list">
            {recentTasks.map((task) => (
              <button key={task.id} type="button" className="compact-list-row" onClick={() => navigate("tasks")}>
                <span className="file-name">{isVideoFile(task.displayName) ? <FileVideo2 size={16} aria-hidden="true" /> : <FileAudio size={16} aria-hidden="true" />}{task.displayName}</span>
                <span className={`status-dot ${task.status}`} aria-hidden="true" />
                <span>{getTaskStatusLabel(task.status)}</span>
                <time dateTime={task.updatedAt}>{formatRelativeDate(task.updatedAt)}</time>
              </button>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
