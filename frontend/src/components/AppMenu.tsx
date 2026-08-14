import {
  Bot,
  CheckCircle2,
  CircleHelp,
  Download,
  FileAudio,
  FolderCog,
  Info,
  LoaderCircle,
  RefreshCw,
  RotateCw,
  ShieldCheck,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import packageMetadata from "../../../package.json";
import type { AvailableUpdate, UpdateDownloadProgress, UpdateService } from "../services/updateService";

type UtilityPanel = "help" | "about" | null;
type ManualUpdatePhase = "idle" | "checking" | "current" | "available" | "downloading" | "restarting" | "error";

interface AppMenuProps {
  updateService: UpdateService | null;
}

interface UtilityDialogProps {
  open: boolean;
  titleId: string;
  closeLabel: string;
  busy?: boolean;
  className?: string;
  children: ReactNode;
  onClose: () => void;
}

/** 把更新下载字节转换为供关于窗口展示的整数百分比。 */
function getProgressPercent(progress: UpdateDownloadProgress): number | null {
  if (!progress.totalBytes || progress.totalBytes <= 0) return null;
  return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
}

/** 提供桌面应用左上角的帮助与关于命令。 */
export function AppMenu({ updateService }: AppMenuProps) {
  const [activePanel, setActivePanel] = useState<UtilityPanel>(null);

  return (
    <>
      <nav className="app-utility-bar" aria-label="应用菜单">
        <button className="utility-menu-button" type="button" onClick={() => setActivePanel("help")} aria-expanded={activePanel === "help"}>
          <CircleHelp size={14} aria-hidden="true" />帮助
        </button>
        <button className="utility-menu-button" type="button" onClick={() => setActivePanel("about")} aria-expanded={activePanel === "about"}>
          <Info size={14} aria-hidden="true" />关于
        </button>
      </nav>
      <HelpDialog open={activePanel === "help"} onClose={() => setActivePanel(null)} />
      <AboutDialog open={activePanel === "about"} service={updateService} onClose={() => setActivePanel(null)} />
    </>
  );
}

/** 渲染带 Escape 与初始焦点管理的通用信息窗口。 */
function UtilityDialog({ open, titleId, closeLabel, busy = false, className = "", children, onClose }: UtilityDialogProps) {
  const closeButtonRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!open) return undefined;
    closeButtonRef.current?.focus();
    /** 在非忙碌状态下允许 Escape 关闭信息窗口。 */
    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape" && !busy) onClose();
    }
    window.addEventListener("keydown", handleEscape);
    return () => window.removeEventListener("keydown", handleEscape);
  }, [busy, onClose, open]);

  if (!open) return null;

  return (
    <div className="dialog-layer utility-dialog-layer" role="presentation">
      <button className="drawer-backdrop" type="button" aria-label="关闭信息窗口背景" onClick={onClose} disabled={busy} />
      <section className={`utility-dialog ${className}`.trim()} role="dialog" aria-modal="true" aria-labelledby={titleId}>
        <button ref={closeButtonRef} className="icon-button utility-dialog-close" type="button" aria-label={closeLabel} onClick={onClose} disabled={busy}>
          <X size={18} aria-hidden="true" />
        </button>
        {children}
      </section>
    </div>
  );
}

interface HelpDialogProps {
  open: boolean;
  onClose: () => void;
}

/** 汇总本地模型、纪要服务和文件处理的必要使用信息。 */
function HelpDialog({ open, onClose }: HelpDialogProps) {
  return (
    <UtilityDialog open={open} titleId="help-title" closeLabel="关闭帮助" className="help-dialog" onClose={onClose}>
      <header className="utility-dialog-header">
        <span className="utility-dialog-symbol" aria-hidden="true"><CircleHelp size={20} /></span>
        <div><span className="eyebrow">帮助</span><h2 id="help-title">使用帮助</h2><p>完成两项处理配置后即可导入本地媒体文件。</p></div>
      </header>
      <div className="help-topics">
        <section className="help-topic" aria-labelledby="help-asr-title">
          <FolderCog size={18} aria-hidden="true" />
          <div><h3 id="help-asr-title">本地 ASR 模型</h3><p>在“设置 → 本地 ASR”选择直接包含 <code>config.yaml</code>、<code>model.pt</code> 和 <code>tokens.json</code> 的模型目录，然后执行“检查环境”。</p></div>
        </section>
        <section className="help-topic" aria-labelledby="help-llm-title">
          <Bot size={18} aria-hidden="true" />
          <div><h3 id="help-llm-title">纪要生成大模型</h3><p>在“设置 → 大模型”选择服务商和模型，填写 API Key 后先测试连接，再保存配置。</p></div>
        </section>
        <section className="help-topic" aria-labelledby="help-files-title">
          <FileAudio size={18} aria-hidden="true" />
          <div><h3 id="help-files-title">媒体文件与数据</h3><p>软件只处理你主动导入的离线文件，不采集麦克风或系统音频；本地记录和逐字稿保存在本机。</p></div>
        </section>
      </div>
    </UtilityDialog>
  );
}

interface AboutDialogProps {
  open: boolean;
  service: UpdateService | null;
  onClose: () => void;
}

/** 展示应用版本，并提供签名更新的手动检查与安装入口。 */
function AboutDialog({ open, service, onClose }: AboutDialogProps) {
  const [phase, setPhase] = useState<ManualUpdatePhase>("idle");
  const [availableUpdate, setAvailableUpdate] = useState<AvailableUpdate | null>(null);
  const [progress, setProgress] = useState<UpdateDownloadProgress>({ downloadedBytes: 0, totalBytes: null });
  const [error, setError] = useState<string | null>(null);
  const updateRef = useRef<AvailableUpdate | null>(null);
  const busy = phase === "checking" || phase === "downloading" || phase === "restarting";

  useEffect(() => {
    if (open) return undefined;
    const update = updateRef.current;
    updateRef.current = null;
    if (update) void update.dispose().catch(() => undefined);
    setAvailableUpdate(null);
    setProgress({ downloadedBytes: 0, totalBytes: null });
    setError(null);
    setPhase("idle");
    return undefined;
  }, [open]);

  /** 主动查询签名更新，并释放上一次尚未安装的更新句柄。 */
  async function checkForUpdates() {
    if (!service || busy) return;
    const previousUpdate = updateRef.current;
    updateRef.current = null;
    if (previousUpdate) await previousUpdate.dispose().catch(() => undefined);
    setAvailableUpdate(null);
    setError(null);
    setPhase("checking");
    try {
      const update = await service.checkForUpdate();
      updateRef.current = update;
      setAvailableUpdate(update);
      setPhase(update ? "available" : "current");
    } catch {
      setError("无法检查更新，请确认网络连接后重试");
      setPhase("error");
    }
  }

  /** 下载并安装用户在关于窗口中确认的签名更新。 */
  async function installUpdate() {
    if (!service || !availableUpdate || busy) return;
    setError(null);
    setPhase("downloading");
    try {
      await availableUpdate.downloadAndInstall(setProgress);
      setPhase("restarting");
      await service.relaunch();
    } catch {
      setError("更新下载或安装失败，请检查网络后重试");
      setPhase("error");
    }
  }

  const progressPercent = getProgressPercent(progress);

  return (
    <UtilityDialog open={open} titleId="about-title" closeLabel="关闭关于" busy={busy} className="about-dialog" onClose={onClose}>
      <header className="about-product">
        <img src="/favicon.svg" alt="听见纪要 Logo" />
        <div><span className="eyebrow">Windows 桌面应用</span><h2 id="about-title">听见纪要</h2><p>版本 {packageMetadata.version}</p></div>
      </header>
      <div className="about-description">
        <ShieldCheck size={18} aria-hidden="true" />
        <p>只处理用户主动导入的离线媒体文件；更新包下载后会先验证签名。</p>
      </div>
      <section className="manual-update" aria-labelledby="manual-update-title" aria-live="polite">
        <div className="manual-update-heading">
          <div><h3 id="manual-update-title">软件更新</h3><p>{service ? "从官方发布源检查新版本" : "当前环境不支持应用内更新"}</p></div>
          <button className="button secondary" type="button" onClick={() => void checkForUpdates()} disabled={!service || busy}>
            {phase === "checking" ? <LoaderCircle className="spin" size={15} aria-hidden="true" /> : <RefreshCw size={15} aria-hidden="true" />}
            {phase === "checking" ? "正在检查" : "检查更新"}
          </button>
        </div>
        {phase === "current" ? <div className="update-result success"><CheckCircle2 size={16} aria-hidden="true" />已是最新版本</div> : null}
        {availableUpdate && ["available", "downloading", "restarting", "error"].includes(phase) ? (
          <div className={`update-result${phase === "error" ? " error" : ""}`}>
            <span><strong>发现新版本 {availableUpdate.version}</strong><small>{phase === "downloading" ? progressPercent === null ? "正在下载更新…" : `正在下载 ${progressPercent}%` : phase === "restarting" ? "更新已安装，正在重新启动…" : error ?? `当前版本 ${availableUpdate.currentVersion}`}</small></span>
            <button className="button primary" type="button" onClick={() => void installUpdate()} disabled={busy}>
              {phase === "downloading" || phase === "restarting" ? <LoaderCircle className="spin" size={15} aria-hidden="true" /> : phase === "error" ? <RotateCw size={15} aria-hidden="true" /> : <Download size={15} aria-hidden="true" />}
              {phase === "error" ? "重试安装" : phase === "downloading" ? "下载中" : phase === "restarting" ? "正在重启" : "下载并安装"}
            </button>
          </div>
        ) : null}
        {phase === "error" && !availableUpdate ? <div className="update-result error">{error}</div> : null}
      </section>
    </UtilityDialog>
  );
}
