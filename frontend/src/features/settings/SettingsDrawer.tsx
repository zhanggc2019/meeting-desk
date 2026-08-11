import { CheckCircle2, ChevronDown, ClipboardCopy, KeyRound, LoaderCircle, Save, Server, ShieldCheck, X } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import type {
  ProviderKind,
  ProviderPresetId,
  ProviderSettingsInput,
  PublicProviderSettings,
  PublicSettings,
} from "../../contracts/desktop";
import { useDesktopClient } from "../../services/DesktopClientContext";
import { getSafeErrorMessage } from "../../services/desktopClient";
import { useAppStore } from "../../stores/appStore";

type ProviderTarget = "transcription" | "minutes";

interface ProviderDraft extends ProviderSettingsInput {
  apiKey: string;
}

interface ProviderPresetDefinition {
  id: ProviderPresetId;
  label: string;
  kind: ProviderKind;
  endpoint: string;
  defaultModel: string;
  models: ReadonlyArray<{ value: string; label: string }>;
  description: string;
}

const LOCAL_FUNASR_ENDPOINT = "local://model/SenseVoiceSmall";
const LOCAL_MODEL_DOWNLOAD_COMMAND = ".\\.venv\\Scripts\\python.exe -c \"from modelscope import snapshot_download; snapshot_download('iic/SenseVoiceSmall', local_dir='model/SenseVoiceSmall')\"";
const XIAOMI_MIMO_LLM_ENDPOINT = "https://api.xiaomimimo.com/v1/chat/completions";
const DEEPSEEK_ENDPOINT = "https://api.deepseek.com/chat/completions";
const ALIYUN_BAILIAN_ENDPOINT = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";

const TRANSCRIPTION_PRESETS: ReadonlyArray<ProviderPresetDefinition> = [
  {
    id: "local_funasr",
    label: "本地 SenseVoiceSmall",
    kind: "local_funasr",
    endpoint: LOCAL_FUNASR_ENDPOINT,
    defaultModel: "SenseVoiceSmall",
    models: [
      { value: "SenseVoiceSmall", label: "SenseVoiceSmall" },
    ],
    description: "从 model/SenseVoiceSmall 加载模型并在本机 CPU 完成转写。",
  },
];

const MINUTES_PRESETS: ReadonlyArray<ProviderPresetDefinition> = [
  {
    id: "deepseek",
    label: "DeepSeek",
    kind: "openai_compatible",
    endpoint: DEEPSEEK_ENDPOINT,
    defaultModel: "deepseek-v4-flash",
    models: [
      { value: "deepseek-v4-flash", label: "deepseek-v4-flash（推荐）" },
      { value: "deepseek-v4-pro", label: "deepseek-v4-pro" },
    ],
    description: "使用 DeepSeek 官方 OpenAI-compatible 接口。",
  },
  {
    id: "xiaomi_mimo_llm",
    label: "Xiaomi MiMo 大模型",
    kind: "openai_compatible",
    endpoint: XIAOMI_MIMO_LLM_ENDPOINT,
    defaultModel: "mimo-v2.5",
    models: [
      { value: "mimo-v2.5", label: "mimo-v2.5（推荐）" },
      { value: "mimo-v2.5-pro", label: "mimo-v2.5-pro" },
    ],
    description: "使用 Xiaomi MiMo 官方 Chat Completions 接口生成结构化会议纪要。",
  },
  {
    id: "aliyun_bailian",
    label: "阿里云百炼（通义千问）",
    kind: "openai_compatible",
    endpoint: ALIYUN_BAILIAN_ENDPOINT,
    defaultModel: "qwen-plus",
    models: [
      { value: "qwen-plus", label: "qwen-plus（推荐）" },
      { value: "qwen-flash", label: "qwen-flash（经济快速）" },
      { value: "qwen-max", label: "qwen-max" },
    ],
    description: "使用阿里云百炼 OpenAI-compatible Chat Completions 接口。",
  },
  {
    id: "custom_openai_compatible",
    label: "第三方 OpenAI Chat Completions",
    kind: "openai_compatible",
    endpoint: "",
    defaultModel: "",
    models: [],
    description: "填写第三方兼容 OpenAI Chat Completions 的完整请求地址和模型名。",
  },
];

/** 判断预设是否允许用户填写完整请求地址和任意模型名。 */
function isCustomPreset(presetId: ProviderPresetId): boolean {
  return presetId === "custom_openai_compatible";
}

/** 返回指定业务类型允许使用的受信任预设。 */
function getPresetDefinitions(target: ProviderTarget): ReadonlyArray<ProviderPresetDefinition> {
  return target === "transcription" ? TRANSCRIPTION_PRESETS : MINUTES_PRESETS;
}

/** 返回指定预设定义；运行时遇到旧值时回退到该业务类型的默认真实预设。 */
function getPresetDefinition(target: ProviderTarget, presetId: ProviderPresetId): ProviderPresetDefinition {
  const definitions = getPresetDefinitions(target);
  return definitions.find((preset) => preset.id === presetId) ?? definitions[0];
}

/** 从旧版公开字段推断预设，确保旧桌面壳仍能打开设置。 */
function inferPresetId(settings: PublicProviderSettings, target: ProviderTarget): ProviderPresetId {
  const allowed = getPresetDefinitions(target).some((preset) => preset.id === settings.presetId);
  if (settings.presetId && allowed) return settings.presetId;
  if (target === "transcription") return "local_funasr";
  if (settings.kind === "mock") return "deepseek";
  if (target === "minutes" && settings.endpoint === XIAOMI_MIMO_LLM_ENDPOINT) return "xiaomi_mimo_llm";
  if (target === "minutes" && settings.endpoint === DEEPSEEK_ENDPOINT) return "deepseek";
  if (target === "minutes" && settings.endpoint === ALIYUN_BAILIAN_ENDPOINT) return "aliyun_bailian";
  return "custom_openai_compatible";
}

/** 将公开设置转换为不会回填密钥的表单草稿。 */
function toDraft(settings: PublicProviderSettings, target: ProviderTarget): ProviderDraft {
  const presetId = inferPresetId(settings, target);
  const preset = getPresetDefinition(target, presetId);
  const managedModel = preset.models.some((model) => model.value === settings.model)
    ? settings.model
    : preset.defaultModel;
  return {
    presetId,
    kind: preset.kind,
    endpoint: isCustomPreset(presetId) ? settings.endpoint : preset.endpoint,
    model: isCustomPreset(presetId) ? settings.model : managedModel,
    apiKey: "",
    connectTimeoutMs: settings.connectTimeoutMs,
    requestTimeoutMs: settings.requestTimeoutMs,
    maxRetries: settings.maxRetries,
  };
}

/** 返回设置抽屉在后端响应前使用的安全真实服务默认值。 */
function getEmptySettings(): PublicSettings {
  const common = {
    secretConfigured: false,
    connectTimeoutMs: 10_000,
    requestTimeoutMs: 120_000,
    maxRetries: 2,
  };
  return {
    transcription: {
      ...common,
      requestTimeoutMs: 3_600_000,
      maxRetries: 0,
      presetId: "local_funasr",
      kind: "local_funasr",
      endpoint: LOCAL_FUNASR_ENDPOINT,
      model: "SenseVoiceSmall",
    },
    minutes: {
      ...common,
      presetId: "deepseek",
      kind: "openai_compatible",
      endpoint: DEEPSEEK_ENDPOINT,
      model: "deepseek-v4-flash",
    },
  };
}

/** 切换预设时填充受信任地址、推荐模型和对应传输类型。 */
function applyPreset(current: ProviderDraft, target: ProviderTarget, presetId: ProviderPresetId): ProviderDraft {
  const preset = getPresetDefinition(target, presetId);
  return {
    ...current,
    presetId: preset.id,
    kind: preset.kind,
    endpoint: preset.endpoint,
    model: preset.defaultModel,
    apiKey: "",
  };
}

/** 判断已保存密钥是否属于当前草稿所选预设。 */
function isSecretConfiguredForDraft(
  settings: PublicProviderSettings,
  draft: ProviderDraft,
  target: ProviderTarget,
): boolean {
  if (!settings.secretConfigured || inferPresetId(settings, target) !== draft.presetId) return false;
  return !isCustomPreset(draft.presetId) || settings.endpoint.trim() === draft.endpoint.trim();
}

interface ConnectionTestState {
  testing: boolean;
  result: { ok: boolean; message: string } | null;
}

/** 创建两个 Provider 相互独立的连接测试初始状态。 */
function getEmptyConnectionTests(): Record<ProviderTarget, ConnectionTestState> {
  return {
    transcription: { testing: false, result: null },
    minutes: { testing: false, result: null },
  };
}

/** 渲染只展示公开配置状态的 Provider 设置抽屉。 */
export function SettingsDrawer() {
  const client = useDesktopClient();
  const open = useAppStore((state) => state.settingsOpen);
  const close = useAppStore((state) => state.closeSettings);
  const markSettingsUpdated = useAppStore((state) => state.markSettingsUpdated);
  const [publicSettings, setPublicSettings] = useState<PublicSettings>(getEmptySettings);
  const [transcription, setTranscription] = useState<ProviderDraft>(() => toDraft(getEmptySettings().transcription, "transcription"));
  const [minutes, setMinutes] = useState<ProviderDraft>(() => toDraft(getEmptySettings().minutes, "minutes"));
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [connectionTests, setConnectionTests] = useState(getEmptyConnectionTests);

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    setError(null);
    setConnectionTests(getEmptyConnectionTests());
    client.getPublicSettings()
      .then((settings) => {
        setPublicSettings(settings);
        setTranscription(toDraft(settings.transcription, "transcription"));
        setMinutes(toDraft(settings.minutes, "minutes"));
      })
      .catch((reason: unknown) => setError(getSafeErrorMessage(reason)))
      .finally(() => setLoading(false));
  }, [client, open]);

  useEffect(() => {
    if (!open) return;
    /** 允许 Escape 关闭非危险设置抽屉。 */
    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape" && !saving) close();
    }
    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [close, open, saving]);

  /** 保存 Provider 配置，响应只接收公开状态。 */
  async function handleSave(event: FormEvent) {
    event.preventDefault();
    setSaving(true);
    setError(null);
    setNotice(null);
    try {
      const saved = await client.saveProviderSettings({ transcription, minutes });
      setPublicSettings(saved);
      setTranscription(toDraft(saved.transcription, "transcription"));
      setMinutes(toDraft(saved.minutes, "minutes"));
      setNotice("设置已保存");
      setConnectionTests(getEmptyConnectionTests());
      markSettingsUpdated();
    } catch (reason) {
      setError(getSafeErrorMessage(reason));
    } finally {
      setSaving(false);
    }
  }

  /** 测试指定 Provider 的安全连接状态。 */
  async function testConnection(target: ProviderTarget) {
    setError(null);
    setNotice(null);
    setConnectionTests((current) => ({
      ...current,
      [target]: { testing: true, result: null },
    }));
    try {
      const saved = await client.saveProviderSettings({ transcription, minutes });
      setPublicSettings(saved);
      setTranscription(toDraft(saved.transcription, "transcription"));
      setMinutes(toDraft(saved.minutes, "minutes"));
      markSettingsUpdated();
      const result = await client.testProviderConnection(target);
      setConnectionTests((current) => ({
        ...current,
        [target]: { testing: false, result: { ok: result.ok, message: result.safeMessage } },
      }));
    } catch (reason) {
      setConnectionTests((current) => ({
        ...current,
        [target]: { testing: false, result: { ok: false, message: getSafeErrorMessage(reason) } },
      }));
    }
  }

  /** 更新一个 Provider 草稿并清除该区块已经过期的连接结果。 */
  function updateProvider(target: ProviderTarget, value: ProviderDraft) {
    if (target === "transcription") setTranscription(value);
    else setMinutes(value);
    setConnectionTests((current) => ({
      ...current,
      [target]: { testing: false, result: null },
    }));
  }

  if (!open) return null;

  return (
    <div className="drawer-layer" role="presentation">
      <button className="drawer-backdrop" type="button" aria-label="关闭设置" onClick={close} />
      <aside className="settings-drawer" role="dialog" aria-modal="true" aria-labelledby="settings-title">
        <header className="drawer-header">
          <div><span className="eyebrow">安全配置</span><h2 id="settings-title">服务设置</h2><p>语音在本机转写；会议纪要服务密钥保存后不会回显。</p></div>
          <button className="icon-button" type="button" aria-label="关闭设置" onClick={close} disabled={saving}><X size={18} /></button>
        </header>

        {loading ? <div className="loading-state"><LoaderCircle className="spin" size={18} aria-hidden="true" />正在读取公开设置…</div> : (
          <form className="settings-form" onSubmit={handleSave}>
            <div className="settings-form-scroll">
              <ProviderSection
                target="transcription"
                title="语音转写"
                description="接收离线音频或视频并返回完整逐字稿"
                value={transcription}
                secretConfigured={isSecretConfiguredForDraft(publicSettings.transcription, transcription, "transcription")}
                connectionTest={connectionTests.transcription}
                onChange={(value) => updateProvider("transcription", value)}
                onTest={() => void testConnection("transcription")}
              />
              <ProviderSection
                target="minutes"
                title="会议纪要"
                description="把逐字稿转换为经过校验的结构化纪要"
                value={minutes}
                secretConfigured={isSecretConfiguredForDraft(publicSettings.minutes, minutes, "minutes")}
                connectionTest={connectionTests.minutes}
                onChange={(value) => updateProvider("minutes", value)}
                onTest={() => void testConnection("minutes")}
              />

              {error ? <div className="inline-alert error" role="alert">{error}</div> : null}
              {notice ? <div className="inline-alert success" role="status"><CheckCircle2 size={16} aria-hidden="true" />{notice}</div> : null}
            </div>

            <div className="drawer-actions">
              <button className="button secondary" type="button" onClick={close} disabled={saving}>取消</button>
              <button className="button primary" type="submit" disabled={saving}><Save size={16} aria-hidden="true" />{saving ? "正在保存" : "保存设置"}</button>
            </div>
          </form>
        )}
      </aside>
    </div>
  );
}

interface ProviderSectionProps {
  target: ProviderTarget;
  title: string;
  description: string;
  value: ProviderDraft;
  secretConfigured: boolean;
  connectionTest: ConnectionTestState;
  onChange: (value: ProviderDraft) => void;
  onTest: () => void;
}

/** 渲染单个 Provider 的预设配置字段，不回显已有密钥。 */
function ProviderSection({ target, title, description, value, secretConfigured, connectionTest, onChange, onTest }: ProviderSectionProps) {
  const presets = getPresetDefinitions(target);
  const selectedPreset = getPresetDefinition(target, value.presetId);
  const isCustom = isCustomPreset(selectedPreset.id);
  const isLocal = selectedPreset.kind === "local_funasr";
  const [copyStatus, setCopyStatus] = useState<"idle" | "copied" | "error">("idle");

  /** 更新一个 Provider 草稿字段。 */
  function update<K extends keyof ProviderDraft>(key: K, nextValue: ProviderDraft[K]) {
    onChange({ ...value, [key]: nextValue });
  }

  /** 切换供应商预设并自动恢复该预设的可信默认值。 */
  function handlePresetChange(nextPresetId: ProviderPresetId) {
    onChange(applyPreset(value, target, nextPresetId));
  }

  /** 将 Windows PowerShell 模型下载命令复制到系统剪贴板。 */
  async function copyModelDownloadCommand() {
    try {
      await navigator.clipboard.writeText(LOCAL_MODEL_DOWNLOAD_COMMAND);
      setCopyStatus("copied");
    } catch {
      setCopyStatus("error");
    }
  }

  return (
    <section className="provider-section" aria-labelledby={`${target}-provider-title`}>
      <div className="provider-title"><Server size={18} aria-hidden="true" /><div><h3 id={`${target}-provider-title`}>{title}</h3><p>{description}</p></div></div>
      <div className="settings-grid">
        <label className="field full-field">服务商
          <select
            aria-label={`${title}服务商`}
            value={value.presetId}
            onChange={(event) => handlePresetChange(event.target.value as ProviderPresetId)}
          >
            {presets.map((preset) => <option key={preset.id} value={preset.id}>{preset.label}</option>)}
          </select>
          <small className="field-help">{selectedPreset.description}</small>
        </label>

        {!isCustom ? (
          <div className="trusted-endpoint full-field" role="note">
            <ShieldCheck size={16} aria-hidden="true" />
            {isLocal
              ? <span><strong>模型保留在本机</strong><small>读取 model/SenseVoiceSmall，不上传音频且无需 API Key。</small></span>
              : <span><strong>官方地址由软件维护</strong><small>无需填写 Base URL，保存时由桌面端校验并使用受信任地址。</small></span>}
          </div>
        ) : null}

        {isLocal ? (
          <div className="local-model-guide full-field" role="note">
            <div className="local-model-guide-header">
              <div>
                <strong>首次使用请下载本地模型</strong>
                <p>下载来源：ModelScope <code>iic/SenseVoiceSmall</code></p>
              </div>
              <button className="button secondary" type="button" onClick={() => void copyModelDownloadCommand()}>
                <ClipboardCopy size={15} aria-hidden="true" />复制下载命令
              </button>
            </div>
            <p>在软件目录打开 PowerShell 并运行命令，模型应放在 <code>model\SenseVoiceSmall</code>。</p>
            <p>下载完成后，目录中至少应有 <code>config.yaml</code>、<code>model.pt</code> 和 <code>tokens.json</code>。</p>
            {copyStatus === "copied" ? <span className="copy-status success" role="status">下载命令已复制</span> : null}
            {copyStatus === "error" ? <span className="copy-status error" role="alert">复制失败，请确认系统允许访问剪贴板</span> : null}
          </div>
        ) : null}

        {selectedPreset.models.length > 0 ? (
          <label className="field full-field">模型
            <select aria-label={`${title}模型`} value={value.model} onChange={(event) => update("model", event.target.value)}>
              {selectedPreset.models.map((model) => <option key={model.value} value={model.value}>{model.label}</option>)}
            </select>
          </label>
        ) : null}

        {isCustom ? (
          <>
            <label className="field full-field">服务地址
              <input aria-label={`${title}服务地址`} value={value.endpoint} onChange={(event) => update("endpoint", event.target.value)} placeholder="https://…" required />
            </label>
            <label className="field full-field">模型
              <input aria-label={`${title}模型`} value={value.model} onChange={(event) => update("model", event.target.value)} placeholder="输入已验证的模型名" required />
            </label>
          </>
        ) : null}

        {!isLocal ? (
          <label className="field full-field">API Key
            <span className="secret-input"><KeyRound size={16} aria-hidden="true" /><input aria-label={`${title} API Key`} value={value.apiKey} onChange={(event) => update("apiKey", event.target.value)} type="password" autoComplete="new-password" placeholder={secretConfigured ? "已安全保存；留空表示不替换" : "输入后交由 Windows 凭据管理器保存"} /></span>
          </label>
        ) : null}
      </div>

      <details className="advanced-settings">
        <summary><ChevronDown size={15} aria-hidden="true" />高级设置</summary>
        <div className="settings-grid advanced-settings-grid">
          {!isLocal ? <label className="field">连接超时（毫秒）<input type="number" min={1000} max={60000} value={value.connectTimeoutMs} onChange={(event) => update("connectTimeoutMs", Number(event.target.value))} /></label> : null}
          <label className="field">请求超时（毫秒）<input type="number" min={5000} max={isLocal ? 14_400_000 : 600_000} value={value.requestTimeoutMs} onChange={(event) => update("requestTimeoutMs", Number(event.target.value))} /></label>
          {!isLocal ? <label className="field">失败重试次数<input type="number" min={0} max={5} value={value.maxRetries} onChange={(event) => update("maxRetries", Number(event.target.value))} /></label> : null}
        </div>
      </details>

      <div className="provider-footer">
        <span className={`secret-status${isLocal || secretConfigured ? " configured" : ""}`}>{isLocal ? "本地模式" : (secretConfigured ? "密钥已配置" : "尚未配置密钥")}</span>
        <button className="button quiet connection-test-button" type="button" onClick={onTest} disabled={connectionTest.testing}>
          {connectionTest.testing ? <><LoaderCircle className="spin" size={15} aria-hidden="true" />正在检查</> : (isLocal ? "检查环境" : "测试连接")}
        </button>
      </div>
      {connectionTest.result ? (
        <div className={`connection-result ${connectionTest.result.ok ? "success" : "error"}`} role={connectionTest.result.ok ? "status" : "alert"}>
          {connectionTest.result.ok ? <CheckCircle2 size={15} aria-hidden="true" /> : null}
          <span>{connectionTest.result.message}</span>
        </div>
      ) : null}
    </section>
  );
}
