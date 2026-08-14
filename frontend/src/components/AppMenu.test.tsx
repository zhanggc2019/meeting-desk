import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AvailableUpdate, UpdateService } from "../services/updateService";
import { AppMenu } from "./AppMenu";

/** 创建可观察手动下载、释放资源和重启行为的更新服务。 */
function createManualUpdateFixture() {
  const dispose = vi.fn().mockResolvedValue(undefined);
  const downloadAndInstall = vi.fn(async (onProgress: Parameters<AvailableUpdate["downloadAndInstall"]>[0]) => {
    onProgress({ downloadedBytes: 50, totalBytes: 100 });
    onProgress({ downloadedBytes: 100, totalBytes: 100 });
  });
  const update: AvailableUpdate = {
    currentVersion: "0.3.3",
    version: "0.3.4",
    date: "2026-08-14T00:00:00Z",
    downloadAndInstall,
    dispose,
  };
  const service: UpdateService = {
    checkForUpdate: vi.fn().mockResolvedValue(update),
    relaunch: vi.fn().mockResolvedValue(undefined),
  };
  return { service, downloadAndInstall };
}

describe("AppMenu", () => {
  it("允许从关于窗口手动下载签名更新并重启", async () => {
    const user = userEvent.setup();
    const fixture = createManualUpdateFixture();
    render(<AppMenu updateService={fixture.service} />);

    await user.click(screen.getByRole("button", { name: "关于" }));
    await user.click(screen.getByRole("button", { name: "检查更新" }));
    expect(await screen.findByText("发现新版本 0.3.4")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "下载并安装" }));
    await waitFor(() => expect(fixture.downloadAndInstall).toHaveBeenCalledOnce());
    await waitFor(() => expect(fixture.service.relaunch).toHaveBeenCalledOnce());
  });

  it("手动检查失败时展示安全错误并允许重试", async () => {
    const user = userEvent.setup();
    const service: UpdateService = {
      checkForUpdate: vi.fn().mockRejectedValue(new Error("internal network detail")),
      relaunch: vi.fn().mockResolvedValue(undefined),
    };
    render(<AppMenu updateService={service} />);

    await user.click(screen.getByRole("button", { name: "关于" }));
    await user.click(screen.getByRole("button", { name: "检查更新" }));

    expect(await screen.findByText("无法检查更新，请确认网络连接后重试")).toBeInTheDocument();
    expect(screen.queryByText("internal network detail")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "检查更新" })).toBeEnabled();
  });
});
