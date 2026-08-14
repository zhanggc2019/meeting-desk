import { readFileSync, statSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("发布品牌资源", () => {
  it("将蓝色麦克风 Logo 放入 Vite 公共资源目录", () => {
    const favicon = readFileSync(resolve(process.cwd(), "frontend/public/favicon.svg"), "utf8");

    expect(favicon).toContain('stop-color="#2563eb"');
    expect(favicon).toContain("<!-- Microphone body -->");
    expect(favicon).toContain("<!-- Microphone stand -->");
  });

  it("为任务队列删除操作提供不会被通用按钮覆盖的专用样式", () => {
    const styles = readFileSync(resolve(process.cwd(), "frontend/src/styles.css"), "utf8");

    expect(styles).toContain(
      ".button.table-action.delete-action { color: #b42318; background: #fff7f6; border-color: #f0b7b2; }",
    );
  });

  it("Tauri 配置引用存在且包含多尺寸的 Windows 图标", () => {
    const config = JSON.parse(
      readFileSync(resolve(process.cwd(), "src-tauri/tauri.conf.json"), "utf8"),
    ) as { bundle: { icon: string[] } };

    for (const iconPath of config.bundle.icon) {
      expect(statSync(resolve(process.cwd(), "src-tauri", iconPath)).size).toBeGreaterThan(0);
    }

    const windowsIcon = readFileSync(resolve(process.cwd(), "src-tauri/icons/icon.ico"));
    expect(windowsIcon.readUInt16LE(0)).toBe(0);
    expect(windowsIcon.readUInt16LE(2)).toBe(1);
    expect(windowsIcon.readUInt16LE(4)).toBeGreaterThanOrEqual(4);
  });
});
