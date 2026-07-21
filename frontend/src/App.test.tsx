import { StrictMode } from "react";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { App } from "./App";
import { createMockDesktopClient } from "./services/mockDesktopClient";
import { useAppStore } from "./stores/appStore";

/** 在每个测试前清理仅用于界面的 Zustand 状态。 */
function resetAppStore() {
  useAppStore.setState({
    page: "workspace",
    selectedMeetingId: null,
    settingsOpen: false,
    settingsRevision: 0,
    taskAttentionCount: 0,
  });
}

/** 创建已配置两个真实预设的离线测试客户端，密钥只存在于测试进程内存。 */
async function createConfiguredClient() {
  const client = createMockDesktopClient();
  await client.saveProviderSettings({
    transcription: {
      presetId: "dashscope_funasr_cn",
      kind: "dashscope_funasr",
      endpoint: "https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription",
      model: "fun-asr",
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

describe("Windows 离线媒体工作台", () => {
  beforeEach(() => {
    resetAppStore();
  });

  it("显示听见纪要品牌 Logo", async () => {
    render(<App client={createMockDesktopClient()} />);

    await screen.findByText("季度复盘.wav");
    const logo = screen.getByRole("img", { name: "听见纪要 Logo" });
    expect(logo).toHaveAttribute("src", "/favicon.svg");
  });

  it("校验单个文件并创建独立处理任务", async () => {
    const user = userEvent.setup();
    render(<App client={await createConfiguredClient()} />);

    const file = new File(["safe mock bytes"], "示例讨论.mp3", { type: "audio/mpeg" });
    const singleFileInput = screen.getByLabelText("选择本地媒体文件");
    await waitFor(() => expect(singleFileInput).toBeEnabled());
    expect(singleFileInput).not.toHaveAttribute("multiple");
    await user.upload(singleFileInput, file);

    expect((await screen.findAllByText("示例讨论.mp3")).length).toBeGreaterThan(0);
    expect(screen.getByText("可处理")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "开始处理" }));

    expect(await screen.findByRole("heading", { name: "任务队列" })).toBeInTheDocument();
    expect((await screen.findAllByText("示例讨论.mp3")).length).toBeGreaterThanOrEqual(2);
  });

  it("对空文件显示校验错误且不允许提交", async () => {
    const user = userEvent.setup();
    render(<App client={await createConfiguredClient()} />);

    const emptyFile = new File([], "空文件.wav", { type: "audio/wav" });
    const input = screen.getByLabelText("选择本地媒体文件");
    await waitFor(() => expect(input).toBeEnabled());
    await user.upload(input, emptyFile);

    expect(await screen.findByText("文件为空")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "开始处理" })).toBeDisabled();
  });

  it("显式切换批量模式并为多个文件创建独立任务", async () => {
    const user = userEvent.setup();
    const client = await createConfiguredClient();
    const createTasks = vi.spyOn(client, "createProcessingTasks");
    render(<App client={client} />);

    await user.click(screen.getByRole("button", { name: "批量处理" }));
    expect(screen.getByText(/单个文件失败不会影响本批次的其他文件/)).toBeInTheDocument();
    expect(screen.getByLabelText("批量选择本地媒体文件")).toHaveAttribute("multiple");
    await waitFor(() => expect(screen.getByLabelText("批量选择本地媒体文件")).toBeEnabled());

    const files = [
      new File(["first safe mock bytes"], "课程上半场.mp3", { type: "audio/mpeg" }),
      new File(["second safe mock bytes"], "课程下半场.wav", { type: "audio/wav" }),
      new File(["video container mock bytes"], "课程录像.mp4", { type: "video/mp4" }),
      new File([], "空录音.wav", { type: "audio/wav" }),
    ];
    await user.upload(screen.getByLabelText("批量选择本地媒体文件"), files);

    expect(await screen.findByRole("heading", { name: "本批次文件" })).toBeInTheDocument();
    expect(screen.getByText("4 个文件")).toBeInTheDocument();
    expect(screen.getByText("3 个可处理")).toBeInTheDocument();
    expect(screen.getByText("1 个校验失败")).toBeInTheDocument();
    expect(screen.getByText("文件为空")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "继续添加" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "单个文件" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "创建 3 个处理任务" }));

    expect(await screen.findByRole("heading", { name: "任务队列" })).toBeInTheDocument();
    expect(createTasks).toHaveBeenCalledOnce();
    expect(createTasks.mock.calls[0]?.[0]).toHaveLength(3);
    expect((await screen.findAllByText("课程上半场.mp3")).length).toBeGreaterThanOrEqual(1);
    expect((await screen.findAllByText("课程下半场.wav")).length).toBeGreaterThanOrEqual(1);
    expect((await screen.findAllByText("课程录像.mp4")).length).toBeGreaterThanOrEqual(1);
  });

  it("取消任务前确认，并只在后端确认后显示已取消", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);
    await user.click(screen.getByRole("button", { name: "任务队列" }));

    const cancelButton = await screen.findByRole("button", { name: "取消" });
    await user.click(cancelButton);
    const dialog = screen.getByRole("alertdialog");
    expect(within(dialog).getByText(/已发送到云端的请求可能继续执行/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "取消任务" }));

    await waitFor(() => expect(screen.getAllByText("已取消").length).toBeGreaterThan(0));
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("打开结构化纪要和完整逐字稿并执行复制", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);
    await user.click(screen.getByRole("button", { name: "会议记录" }));

    expect(await screen.findByRole("columnheader", { name: "录音时长" })).toBeInTheDocument();
    expect(screen.getByRole("columnheader", { name: "总处理耗时" })).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "22:00" })).toBeInTheDocument();
    await user.click(await screen.findByRole("button", { name: /^产品交付节奏讨论/ }));
    expect(await screen.findByRole("heading", { name: "产品交付节奏讨论" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "会议摘要" })).toBeInTheDocument();
    expect(screen.getByText(/处理耗时 22:00/)).toBeInTheDocument();

    await user.click(screen.getByRole("tab", { name: "完整逐字稿" }));
    expect(screen.getAllByText("说话人 A")).toHaveLength(2);
    await user.click(screen.getByRole("button", { name: "复制全文" }));
    expect(await screen.findByText("已复制完整逐字稿")).toBeInTheDocument();
  });

  it("使用受信任预设隐藏地址输入并提供受控模型下拉", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    expect(await screen.findByRole("heading", { name: "服务设置" })).toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("语音转写服务商"), "dashscope_funasr_cn");
    expect(within(screen.getByLabelText("语音转写服务商")).getByRole("option", { name: "阿里云百炼 FunASR（国际 / 新加坡）" })).toBeInTheDocument();
    expect(screen.queryByLabelText("语音转写服务地址")).not.toBeInTheDocument();
    expect(screen.getByLabelText("语音转写模型")).toHaveValue("fun-asr");
    expect(within(screen.getByLabelText("语音转写模型")).getByRole("option", { name: "fun-asr-mtl" })).toBeInTheDocument();
    expect(screen.getAllByText("官方地址由软件维护").length).toBeGreaterThan(0);

    await user.selectOptions(screen.getByLabelText("语音转写服务商"), "xiaomi_mimo_asr");
    expect(screen.queryByLabelText("语音转写服务地址")).not.toBeInTheDocument();
    expect(screen.getByLabelText("语音转写模型")).toHaveValue("mimo-v2.5-asr");

    await user.selectOptions(screen.getByLabelText("语音转写服务商"), "volcengine_asr_flash");
    expect(screen.queryByLabelText("语音转写服务地址")).not.toBeInTheDocument();
    expect(screen.getByLabelText("语音转写模型")).toHaveValue("bigmodel");

    await user.selectOptions(screen.getByLabelText("会议纪要服务商"), "deepseek");
    expect(screen.queryByLabelText("会议纪要服务地址")).not.toBeInTheDocument();
    expect(screen.getByLabelText("会议纪要模型")).toHaveValue("deepseek-v4-flash");
    expect(within(screen.getByLabelText("会议纪要模型")).getByRole("option", { name: "deepseek-v4-pro" })).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("会议纪要服务商"), "aliyun_bailian");
    expect(screen.queryByLabelText("会议纪要服务地址")).not.toBeInTheDocument();
    expect(screen.getByLabelText("会议纪要模型")).toHaveValue("qwen-plus");
    expect(within(screen.getByLabelText("会议纪要模型")).getByRole("option", { name: "qwen-flash（经济快速）" })).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("会议纪要服务商"), "xiaomi_mimo_llm");
    expect(screen.queryByLabelText("会议纪要服务地址")).not.toBeInTheDocument();
    expect(screen.getByLabelText("会议纪要模型")).toHaveValue("mimo-v2.5");
    expect(within(screen.getByLabelText("会议纪要模型")).getByRole("option", { name: "mimo-v2.5-pro" })).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("会议纪要服务商"), "custom_openai_compatible");
    expect(screen.getByLabelText("会议纪要服务地址")).toBeInTheDocument();
    expect(screen.getByLabelText("会议纪要模型")).toHaveAttribute("placeholder", "输入已验证的模型名");
    expect(screen.queryByRole("option", { name: /Mock/ })).not.toBeInTheDocument();
    expect(screen.queryByRole("option", { name: "第三方 OpenAI Completions（旧版）" })).not.toBeInTheDocument();

  });

  it("保存 MiMo 大模型托管配置", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const saveSettings = vi.spyOn(client, "saveProviderSettings");
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    await user.selectOptions(await screen.findByLabelText("会议纪要服务商"), "xiaomi_mimo_llm");
    await user.selectOptions(screen.getByLabelText("会议纪要模型"), "mimo-v2.5-pro");
    await user.click(screen.getByRole("button", { name: "保存设置" }));
    expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({
      minutes: expect.objectContaining({
        presetId: "xiaomi_mimo_llm",
        kind: "openai_compatible",
        endpoint: "https://api.xiaomimimo.com/v1/chat/completions",
        model: "mimo-v2.5-pro",
      }),
    }));

  });

  it("保存 Xiaomi MiMo 与火山引擎托管转写预设时使用固定字段", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const saveSettings = vi.spyOn(client, "saveProviderSettings");
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    await user.selectOptions(await screen.findByLabelText("语音转写服务商"), "xiaomi_mimo_asr");
    await user.click(screen.getByRole("button", { name: "保存设置" }));
    expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({
      transcription: expect.objectContaining({
        presetId: "xiaomi_mimo_asr",
        kind: "xiaomi_mimo",
        endpoint: "https://api.xiaomimimo.com/v1/chat/completions",
        model: "mimo-v2.5-asr",
      }),
    }));

    await user.selectOptions(screen.getByLabelText("语音转写服务商"), "volcengine_asr_flash");
    await user.click(screen.getByRole("button", { name: "保存设置" }));
    expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({
      transcription: expect.objectContaining({
        presetId: "volcengine_asr_flash",
        kind: "volcengine_asr",
        endpoint: "https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash",
        model: "bigmodel",
      }),
    }));
  });

  it("默认使用 DeepSeek 推荐模型保存参数且不回显密钥", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const saveSettings = vi.spyOn(client, "saveProviderSettings");
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    expect(await screen.findByRole("heading", { name: "服务设置" })).toBeInTheDocument();
    await user.selectOptions(screen.getByLabelText("语音转写服务商"), "dashscope_funasr_cn");
    await user.selectOptions(screen.getByLabelText("会议纪要服务商"), "deepseek");
    expect(screen.getByLabelText("会议纪要模型")).toHaveValue("deepseek-v4-flash");

    const sentinelSecret = "test-only-secret-value";
    await user.type(screen.getByLabelText("语音转写 API Key"), sentinelSecret);
    await user.click(screen.getByRole("button", { name: "保存设置" }));

    expect(await screen.findByText("设置已保存")).toBeInTheDocument();
    expect(screen.queryByDisplayValue(sentinelSecret)).not.toBeInTheDocument();
    expect(screen.getAllByText("密钥已配置")).toHaveLength(1);
    expect(saveSettings).toHaveBeenCalledWith(expect.objectContaining({
      transcription: expect.objectContaining({
        presetId: "dashscope_funasr_cn",
        endpoint: "https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription",
        model: "fun-asr",
      }),
      minutes: expect.objectContaining({
        presetId: "deepseek",
        endpoint: "https://api.deepseek.com/chat/completions",
        model: "deepseek-v4-flash",
      }),
    }));
  });

  it("真实服务未配置时显示双 Provider 引导并可打开设置", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);

    expect(await screen.findByRole("heading", { name: "开始前，请先连接两项服务" })).toBeInTheDocument();
    expect(screen.getByText("FunASR / ASR 转写接口")).toBeInTheDocument();
    expect(screen.getByText("纪要生成大模型接口")).toBeInTheDocument();
    expect(screen.getAllByText("请补充：API Key")).toHaveLength(2);
    expect(screen.getByRole("button", { name: "选择音频或视频" })).toBeDisabled();
    expect(screen.getByLabelText("选择本地媒体文件")).toBeDisabled();
    expect(screen.getByText("配置服务后选择媒体").closest(".file-dropzone")).toHaveAttribute("aria-disabled", "true");
    expect(screen.queryByText(/Mock/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "打开服务设置" }));
    expect(await screen.findByRole("heading", { name: "服务设置" })).toBeInTheDocument();
  });

  it("测试连接会在对应服务区块内返回明确结果", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);

    await user.click(screen.getByRole("button", { name: "设置" }));
    const transcriptionSection = await screen.findByRole("region", { name: "语音转写" });
    await user.type(within(transcriptionSection).getByLabelText("语音转写 API Key"), "test-only-connection-key");
    await user.click(within(transcriptionSection).getByRole("button", { name: "测试连接" }));
    expect(await within(transcriptionSection).findByText(/Windows 桌面应用中测试连接/)).toBeInTheDocument();
  });

  it("在会议详情渲染安全的 Markdown 文档预览", async () => {
    const user = userEvent.setup();
    render(<StrictMode><App client={createMockDesktopClient()} /></StrictMode>);
    await user.click(screen.getByRole("button", { name: "会议记录" }));
    await user.click(await screen.findByRole("button", { name: /^产品交付节奏讨论/ }));

    await user.click(await screen.findByRole("tab", { name: "Markdown 预览" }));
    const panel = await screen.findByRole("tabpanel", { name: "Markdown 文档预览" });
    expect(within(panel).getByRole("heading", { name: "导出文档效果" })).toBeInTheDocument();
    expect(await within(panel).findByRole("heading", { name: "产品交付节奏讨论" })).toBeInTheDocument();
    expect(within(panel).getByRole("table")).toBeInTheDocument();
    expect(within(panel).queryByText("正在生成预览…")).not.toBeInTheDocument();
  });

  it("界面不提供实时音频采集入口", async () => {
    render(<App client={createMockDesktopClient()} />);
    await screen.findByText("季度复盘.wav");
    expect(screen.queryByText("麦克风")).not.toBeInTheDocument();
    expect(screen.queryByText("系统声音")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "开始录音" })).not.toBeInTheDocument();
  });
});
