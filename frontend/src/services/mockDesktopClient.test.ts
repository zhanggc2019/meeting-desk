import { describe, expect, it } from "vitest";
import { createMockDesktopClient } from "./mockDesktopClient";

describe("Mock DesktopClient", () => {
  it("按显式导入模式返回单文件或批量候选项", async () => {
    const client = createMockDesktopClient();

    await expect(client.selectAudioFiles("single")).resolves.toHaveLength(1);
    await expect(client.selectAudioFiles("batch")).resolves.toHaveLength(2);
  });

  it("只返回公开配置状态而不回显 API Key", async () => {
    const client = createMockDesktopClient();
    const saved = await client.saveProviderSettings({
      transcription: {
        presetId: "custom_openai_compatible",
        kind: "openai_compatible",
        endpoint: "https://example.test/asr",
        model: "asr-test",
        apiKey: "test-only-secret-value",
        connectTimeoutMs: 5000,
        requestTimeoutMs: 60_000,
        maxRetries: 1,
      },
      minutes: {
        presetId: "mock",
        kind: "mock",
        endpoint: "",
        model: "minutes-test",
        connectTimeoutMs: 5000,
        requestTimeoutMs: 60_000,
        maxRetries: 1,
      },
    });

    expect(saved.transcription.secretConfigured).toBe(true);
    expect(saved.transcription.presetId).toBe("custom_openai_compatible");
    expect(saved.transcription).not.toHaveProperty("apiKey");
    expect(JSON.stringify(saved)).not.toContain("test-only-secret-value");
  });

  it("不会把未发起网络请求的真实 Provider 误报为连接成功", async () => {
    const client = createMockDesktopClient();
    const mockResult = await client.testProviderConnection("transcription");
    expect(mockResult).toEqual({ ok: true, safeMessage: "Mock 服务可用" });

    await client.saveProviderSettings({
      transcription: {
        presetId: "dashscope_funasr_cn",
        kind: "dashscope_funasr",
        endpoint: "https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription",
        model: "fun-asr",
        connectTimeoutMs: 5000,
        requestTimeoutMs: 60_000,
        maxRetries: 1,
      },
      minutes: {
        presetId: "deepseek",
        kind: "openai_compatible",
        endpoint: "https://api.deepseek.com/chat/completions",
        model: "deepseek-v4-flash",
        apiKey: "test-only-secret-value",
        connectTimeoutMs: 5000,
        requestTimeoutMs: 60_000,
        maxRetries: 1,
      },
    });

    await expect(client.testProviderConnection("transcription")).resolves.toEqual({
      ok: false,
      safeMessage: "请先保存 API Key",
    });
    await expect(client.testProviderConnection("minutes")).resolves.toEqual({
      ok: false,
      safeMessage: "真实 Provider 字段尚未完成最小验证，未发送网络请求",
    });
  });

  it("逐项校验批量文件且保留部分成功", async () => {
    const client = createMockDesktopClient();
    const candidates = await client.registerBrowserFiles([
      { name: "可用文件.mp3", size: 1024, type: "audio/mpeg" },
      { name: "空文件.wav", size: 0, type: "audio/wav" },
      { name: "说明.txt", size: 100, type: "text/plain" },
    ]);

    expect(candidates.map((candidate) => candidate.validationStatus)).toEqual(["ready", "invalid", "invalid"]);
    expect(candidates.filter((candidate) => candidate.artifactId)).toHaveLength(1);
  });

  it("返回完整模板注册表和 Markdown 预览", async () => {
    const client = createMockDesktopClient();
    const registry = await client.listMinutesTemplates();
    const preview = await client.getMeetingMarkdownPreview("meeting-demo-1");

    expect(registry.map((template) => template.id)).toEqual(expect.arrayContaining([
      "adaptive",
      "course_summary",
      "research_project",
      "academic_lecture",
      "profile_interview",
      "in_depth_interview",
      "business_plan",
      "article_outline",
    ]));
    expect(preview).toContain("# 产品交付节奏讨论");
    expect(preview).toContain("| 事项 | 负责人 | 截止日期 |");
  });
});
