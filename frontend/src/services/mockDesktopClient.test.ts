import { describe, expect, it } from "vitest";
import { createMockDesktopClient } from "./mockDesktopClient";

/** 为文件导入测试配置两个真实 Provider 预设。 */
async function createConfiguredClient() {
  const client = createMockDesktopClient();
  await client.saveProviderSettings({
    transcription: {
      presetId: "xiaomi_mimo_asr",
      kind: "xiaomi_mimo",
      endpoint: "https://api.xiaomimimo.com/v1/chat/completions",
      model: "mimo-v2.5-asr",
      apiKey: "test-transcription-key",
      connectTimeoutMs: 5000,
      requestTimeoutMs: 60_000,
      maxRetries: 1,
    },
    minutes: {
      presetId: "deepseek",
      kind: "openai_compatible",
      endpoint: "https://api.deepseek.com/chat/completions",
      model: "deepseek-v4-flash",
      apiKey: "test-minutes-key",
      connectTimeoutMs: 5000,
      requestTimeoutMs: 60_000,
      maxRetries: 1,
    },
  });
  return client;
}

describe("浏览器测试 DesktopClient", () => {
  it("按显式导入模式返回单文件或批量候选项", async () => {
    const client = await createConfiguredClient();

    await expect(client.selectAudioFiles("single")).resolves.toHaveLength(1);
    await expect(client.selectAudioFiles("batch")).resolves.toHaveLength(2);
  });

  it("只返回公开配置状态而不回显 API Key", async () => {
    const client = createMockDesktopClient();
    const saved = await client.saveProviderSettings({
      transcription: {
        presetId: "xiaomi_mimo_asr",
        kind: "xiaomi_mimo",
        endpoint: "https://api.xiaomimimo.com/v1/chat/completions",
        model: "mimo-v2.5-asr",
        apiKey: "test-only-secret-value",
        connectTimeoutMs: 5000,
        requestTimeoutMs: 60_000,
        maxRetries: 1,
      },
      minutes: {
        presetId: "deepseek",
        kind: "openai_compatible",
        endpoint: "https://api.deepseek.com/chat/completions",
        model: "deepseek-v4-flash",
        connectTimeoutMs: 5000,
        requestTimeoutMs: 60_000,
        maxRetries: 1,
      },
    });

    expect(saved.transcription.secretConfigured).toBe(true);
    expect(saved.transcription.presetId).toBe("xiaomi_mimo_asr");
    expect(saved.transcription).not.toHaveProperty("apiKey");
    expect(JSON.stringify(saved)).not.toContain("test-only-secret-value");
  });

  it("不会把未发起网络请求的真实 Provider 误报为连接成功", async () => {
    const client = createMockDesktopClient();
    const unconfiguredResult = await client.testProviderConnection("transcription");
    expect(unconfiguredResult).toEqual({ ok: false, safeMessage: "请先保存 API Key" });

    await client.saveProviderSettings({
      transcription: {
        presetId: "xiaomi_mimo_asr",
        kind: "xiaomi_mimo",
        endpoint: "https://api.xiaomimimo.com/v1/chat/completions",
        model: "mimo-v2.5-asr",
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
      safeMessage: "浏览器测试环境不发送网络请求，请在 Windows 桌面应用中测试连接",
    });
  });

  it("自定义纪要地址变化时不会沿用旧地址的密钥状态", async () => {
    const client = createMockDesktopClient();
    const customMinutes = {
      presetId: "custom_openai_compatible",
      kind: "openai_compatible",
      model: "test-model",
      connectTimeoutMs: 5000,
      requestTimeoutMs: 60_000,
      maxRetries: 1,
    } as const;
    const transcription = {
      presetId: "xiaomi_mimo_asr",
      kind: "xiaomi_mimo",
      endpoint: "https://api.xiaomimimo.com/v1/chat/completions",
      model: "mimo-v2.5-asr",
      connectTimeoutMs: 5000,
      requestTimeoutMs: 60_000,
      maxRetries: 1,
    } as const;

    await client.saveProviderSettings({
      transcription: { ...transcription, apiKey: "test-transcription-key" },
      minutes: {
        ...customMinutes,
        endpoint: "https://first.example.test/v1/chat/completions",
        apiKey: "test-minutes-key",
      },
    });
    const changed = await client.saveProviderSettings({
      transcription,
      minutes: {
        ...customMinutes,
        endpoint: "https://second.example.test/v1/chat/completions",
      },
    });

    expect(changed.minutes.secretConfigured).toBe(false);
    expect(changed.minutes.ready).toBe(false);
  });

  it("两个服务未配置完成时拒绝注册音频", async () => {
    const client = createMockDesktopClient();
    await expect(client.selectAudioFiles("single")).rejects.toThrow("请先完成语音转写和会议纪要服务配置");
    await expect(client.registerBrowserFiles([
      { name: "不应导入.mp3", size: 1024, type: "audio/mpeg" },
    ])).rejects.toThrow("请先完成语音转写和会议纪要服务配置");
  });

  it("逐项校验批量文件且保留部分成功", async () => {
    const client = await createConfiguredClient();
    const candidates = await client.registerBrowserFiles([
      { name: "可用文件.mp3", size: 1024, type: "audio/mpeg" },
      { name: "课程录像.mp4", size: 2048, type: "video/mp4" },
      { name: "空文件.wav", size: 0, type: "audio/wav" },
      { name: "说明.txt", size: 100, type: "text/plain" },
    ]);

    expect(candidates.map((candidate) => candidate.validationStatus)).toEqual(["ready", "ready", "invalid", "invalid"]);
    expect(candidates[1]?.mimeType).toBe("video/mp4");
    expect(candidates.filter((candidate) => candidate.artifactId)).toHaveLength(2);
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
