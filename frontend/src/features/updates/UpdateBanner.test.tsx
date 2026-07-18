import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { AvailableUpdate, UpdateService } from "../../services/updateService";
import { UpdateBanner } from "./UpdateBanner";

/** 创建可观察下载进度与重启调用的更新服务。 */
function createUpdateFixture() {
  const dispose = vi.fn().mockResolvedValue(undefined);
  const downloadAndInstall = vi.fn(async (onProgress: Parameters<AvailableUpdate["downloadAndInstall"]>[0]) => {
    onProgress({ downloadedBytes: 50, totalBytes: 100 });
    onProgress({ downloadedBytes: 100, totalBytes: 100 });
  });
  const update: AvailableUpdate = {
    currentVersion: "0.1.0",
    version: "0.2.0",
    date: "2026-07-18T00:00:00Z",
    downloadAndInstall,
    dispose,
  };
  const service: UpdateService = {
    checkForUpdate: vi.fn().mockResolvedValue(update),
    relaunch: vi.fn().mockResolvedValue(undefined),
  };
  return { service, update, downloadAndInstall, dispose };
}

describe("UpdateBanner", () => {
  it("静默检查后展示版本，并在用户确认后安装和重启", async () => {
    const user = userEvent.setup();
    const fixture = createUpdateFixture();
    render(<UpdateBanner service={fixture.service} autoCheckDelayMs={0} />);

    expect(await screen.findByText("发现新版本 0.2.0")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "立即更新" }));

    await waitFor(() => expect(fixture.downloadAndInstall).toHaveBeenCalledOnce());
    await waitFor(() => expect(fixture.service.relaunch).toHaveBeenCalledOnce());
  });

  it("没有新版本或检查失败时保持安静", async () => {
    const noUpdate: UpdateService = {
      checkForUpdate: vi.fn().mockResolvedValue(null),
      relaunch: vi.fn().mockResolvedValue(undefined),
    };
    const { rerender } = render(<UpdateBanner service={noUpdate} autoCheckDelayMs={0} />);
    await waitFor(() => expect(noUpdate.checkForUpdate).toHaveBeenCalledOnce());
    expect(screen.queryByLabelText("软件更新")).not.toBeInTheDocument();

    const failed: UpdateService = {
      checkForUpdate: vi.fn().mockRejectedValue(new Error("network unavailable")),
      relaunch: vi.fn().mockResolvedValue(undefined),
    };
    rerender(<UpdateBanner service={failed} autoCheckDelayMs={0} />);
    await waitFor(() => expect(failed.checkForUpdate).toHaveBeenCalledOnce());
    expect(screen.queryByLabelText("软件更新")).not.toBeInTheDocument();
  });
});
