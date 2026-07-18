import { Download, LoaderCircle, RefreshCw, RotateCw, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { AvailableUpdate, UpdateDownloadProgress, UpdateService } from "../../services/updateService";

type UpdatePhase = "hidden" | "checking" | "available" | "downloading" | "restarting" | "error";

interface UpdateBannerProps {
  service: UpdateService | null;
  autoCheckDelayMs?: number;
}
/** 把下载字节转换为稳定的百分比；服务未提供总大小时返回空值。 */
function getProgressPercent(progress: UpdateDownloadProgress): number | null {
  if (!progress.totalBytes || progress.totalBytes <= 0) return null;
  return Math.min(100, Math.round((progress.downloadedBytes / progress.totalBytes) * 100));
}

/** 在启动后静默检查签名更新，并在有新版本时展示轻量操作条。 */
export function UpdateBanner({ service, autoCheckDelayMs = 2_000 }: UpdateBannerProps) {
  const [phase, setPhase] = useState<UpdatePhase>(service ? "checking" : "hidden");
  const [availableUpdate, setAvailableUpdate] = useState<AvailableUpdate | null>(null);
  const [progress, setProgress] = useState<UpdateDownloadProgress>({ downloadedBytes: 0, totalBytes: null });
  const [error, setError] = useState<string | null>(null);
  const updateRef = useRef<AvailableUpdate | null>(null);

  useEffect(() => {
    if (!service) return;
    let active = true;
    const timer = window.setTimeout(() => {
      service.checkForUpdate()
        .then((update) => {
          if (!active) {
            if (update) void update.dispose();
            return;
          }
          updateRef.current = update;
          setAvailableUpdate(update);
          setPhase(update ? "available" : "hidden");
        })
        .catch(() => {
          if (active) setPhase("hidden");
        });
    }, autoCheckDelayMs);

    return () => {
      active = false;
      window.clearTimeout(timer);
      if (updateRef.current) void updateRef.current.dispose();
      updateRef.current = null;
    };
  }, [autoCheckDelayMs, service]);

  /** 下载并安装已经通过签名元数据检查的版本，Windows 安装时可能自动退出。 */
  async function installUpdate() {
    if (!service || !availableUpdate || phase === "downloading") return;
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

  /** 在当前会话忽略此版本，并释放 updater 资源。 */
  async function dismissUpdate() {
    const update = updateRef.current;
    updateRef.current = null;
    setAvailableUpdate(null);
    setPhase("hidden");
    if (update) await update.dispose().catch(() => undefined);
  }

  if (!availableUpdate || phase === "hidden" || phase === "checking") return null;
  const progressPercent = getProgressPercent(progress);

  return (
    <section className={`update-banner ${phase === "error" ? "is-error" : ""}`} aria-live="polite" aria-label="软件更新">
      <div className="update-banner-copy">
        <span className="update-banner-icon" aria-hidden="true">
          {phase === "downloading" || phase === "restarting" ? <LoaderCircle className="spin" size={17} /> : <RefreshCw size={17} />}
        </span>
        <span>
          <strong>{phase === "error" ? "更新没有完成" : `发现新版本 ${availableUpdate.version}`}</strong>
          <small>
            {phase === "downloading"
              ? progressPercent === null ? "正在安全下载更新…" : `正在下载 ${progressPercent}%`
              : phase === "restarting"
                ? "更新已安装，正在重新启动…"
                : error ?? `当前版本 ${availableUpdate.currentVersion} · 更新包将先验证签名`}
          </small>
        </span>
      </div>
      {phase === "downloading" && progressPercent !== null ? (
        <progress className="update-progress" value={progressPercent} max={100} aria-label="更新下载进度" />
      ) : null}
      <div className="update-banner-actions">
        <button className="button primary compact-button" type="button" onClick={() => void installUpdate()} disabled={phase === "downloading" || phase === "restarting"}>
          {phase === "error" ? <RotateCw size={15} aria-hidden="true" /> : <Download size={15} aria-hidden="true" />}
          {phase === "error" ? "重试" : phase === "downloading" ? "下载中" : phase === "restarting" ? "正在重启" : "立即更新"}
        </button>
        <button className="icon-button" type="button" aria-label="稍后更新" onClick={() => void dismissUpdate()} disabled={phase === "downloading" || phase === "restarting"}>
          <X size={16} />
        </button>
      </div>
    </section>
  );
}
