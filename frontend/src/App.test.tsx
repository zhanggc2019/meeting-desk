import { StrictMode } from "react";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
      presetId: "local_funasr",
      kind: "local_funasr",
      endpoint: "local://model/SenseVoiceSmall",
      model: "SenseVoiceSmall",
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
    vi.clearAllMocks();
    resetAppStore();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("显示听见纪要品牌 Logo", async () => {
    render(<App client={createMockDesktopClient()} />);

    await screen.findByText("季度复盘.wav");
    const logo = screen.getByRole("img", { name: "听见纪要 Logo" });
    expect(logo).toHaveAttribute("src", "/favicon.svg");
  });

  it("在左上角提供帮助与关于信息，并允许手动检查更新", async () => {
    const user = userEvent.setup();
    const updateService = {
      checkForUpdate: vi.fn().mockResolvedValue(null),
      relaunch: vi.fn().mockResolvedValue(undefined),
    };
    render(<App client={createMockDesktopClient()} updateService={updateService} />);

    expect(screen.getByRole("button", { name: "帮助" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "帮助" }));
    expect(screen.getByRole("heading", { name: "使用帮助" })).toBeInTheDocument();
    expect(screen.getByText(/同级 fsmn-vad/)).toBeInTheDocument();
    expect(screen.getByText(/最长 30 秒/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "关闭帮助" }));

    await user.click(screen.getByRole("button", { name: "关于" }));
    expect(screen.getByRole("heading", { name: "听见纪要" })).toBeInTheDocument();
    expect(screen.getByText(/^版本 \d+\.\d+\.\d+$/)).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "检查更新" }));
    expect(await screen.findByText("已是最新版本")).toBeInTheDocument();
    expect(updateService.checkForUpdate).toHaveBeenCalledOnce();
  });

  it("将本地 ASR 与大模型配置拆分到独立设置分类", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "本地 ASR" })).toHaveAttribute("aria-current", "page");
    expect(screen.getByRole("region", { name: "本地 ASR 模型" })).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "纪要生成大模型" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存 ASR 设置" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "大模型" }));
    expect(screen.getByRole("button", { name: "大模型" })).toHaveAttribute("aria-current", "page");
    expect(screen.queryByRole("region", { name: "本地 ASR 模型" })).not.toBeInTheDocument();
    expect(screen.getByRole("region", { name: "纪要生成大模型" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "保存大模型设置" })).toBeInTheDocument();
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

  it("存在进行中任务时每三秒自动刷新任务队列", async () => {
    vi.useFakeTimers();
    const client = createMockDesktopClient();
    const listTasks = vi.spyOn(client, "listProcessingTasks");
    render(<App client={client} />);

    fireEvent.click(screen.getByRole("button", { name: "任务队列" }));
    await act(async () => {
      await Promise.resolve();
    });
    const initialLoadCount = listTasks.mock.calls.length;
    expect(initialLoadCount).toBeGreaterThan(0);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3_000);
    });
    expect(listTasks).toHaveBeenCalledTimes(initialLoadCount + 1);
  });

  it("确认后删除失败任务并从队列移除", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);
    await user.click(screen.getByRole("button", { name: "任务队列" }));

    const failedRow = (await screen.findByText("客户访谈.mp3")).closest("tr");
    expect(failedRow).not.toBeNull();
    const deleteButton = within(failedRow!).getByRole("button", { name: "删除" });
    expect(deleteButton).toHaveClass("delete-action");
    await user.click(deleteButton);

    const dialog = screen.getByRole("alertdialog");
    expect(within(dialog).getByText(/只会删除任务记录和受管临时文件/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "删除任务" }));

    await waitFor(() => expect(screen.queryByText("客户访谈.mp3")).not.toBeInTheDocument());
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

  it("确认后删除会议和关联记录并返回列表", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const deleteMeeting = vi.spyOn(client, "deleteMeeting");
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "会议记录" }));
    await user.click(await screen.findByRole("button", { name: /^产品交付节奏讨论/ }));

    await user.click(await screen.findByRole("button", { name: "删除会议" }));
    const dialog = screen.getByRole("alertdialog");
    expect(within(dialog).getByText(/原始文件不会受影响/)).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "删除会议" }));

    await waitFor(() => expect(deleteMeeting).toHaveBeenCalledWith("meeting-demo-1"));
    expect(await screen.findByRole("heading", { name: "会议记录" })).toBeInTheDocument();
    expect(screen.queryByText("产品交付节奏讨论")).not.toBeInTheDocument();
  });

  it("本地 ASR 隐藏地址和密钥并保留受控纪要预设", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();
    expect(within(screen.getByLabelText("语音转写服务商")).getByRole("option", { name: "本地 SenseVoiceSmall" })).toBeInTheDocument();
    expect(screen.queryByLabelText("语音转写服务地址")).not.toBeInTheDocument();
    expect(screen.getByLabelText("语音转写模型")).toHaveValue("SenseVoiceSmall");
    expect(screen.getByText("模型保留在本机")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "大模型" }));
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

  it("默认使用本地 SenseVoiceSmall 且不要求 ASR API Key", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    const transcriptionSection = await screen.findByRole("region", { name: "本地 ASR 模型" });
    expect(within(transcriptionSection).getByLabelText("语音转写服务商")).toHaveValue("local_funasr");
    expect(within(transcriptionSection).getAllByText(/SenseVoiceSmall/).length).toBeGreaterThan(0);
    expect(within(transcriptionSection).queryByLabelText("语音转写 API Key")).not.toBeInTheDocument();
  });

  it("展示模型放置目录和本地 ASR 使用步骤", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    const transcriptionSection = await screen.findByRole("region", { name: "本地 ASR 模型" });
    expect(within(transcriptionSection).getByText(/%LOCALAPPDATA%\\com\.internal\.meetingdesk\\model\\SenseVoiceSmall/)).toBeInTheDocument();
    expect(within(transcriptionSection).getByText(/%LOCALAPPDATA%\\com\.internal\.meetingdesk\\model\\fsmn-vad/)).toBeInTheDocument();
    expect(within(transcriptionSection).getByText(/下载完成后，将完整的 SenseVoiceSmall 文件夹放到/)).toBeInTheDocument();
    expect(within(transcriptionSection).getByText(/最长 30 秒的语音段/)).toBeInTheDocument();
    expect(within(transcriptionSection).getByRole("button", { name: "检查环境" })).toBeInTheDocument();
    expect(within(transcriptionSection).queryByText(/下载来源/)).not.toBeInTheDocument();
    expect(within(transcriptionSection).queryByRole("button", { name: "复制模型下载命令" })).not.toBeInTheDocument();
  });

  it("选择已有 SenseVoiceSmall 目录并随本地转写设置保存", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const selectModelDirectory = vi.spyOn(client, "selectLocalModelDirectory")
      .mockResolvedValue("D:\\Projects\\funasr-demo\\model\\SenseVoiceSmall");
    const saveSettings = vi.spyOn(client, "saveProviderSettings");
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    const transcriptionSection = await screen.findByRole("region", { name: "本地 ASR 模型" });
    const modelPath = within(transcriptionSection).getByLabelText("语音转写模型路径");
    expect(modelPath).toHaveValue("");

    await user.click(within(transcriptionSection).getByRole("button", { name: "选择模型目录" }));
    expect(selectModelDirectory).toHaveBeenCalledOnce();
    expect(modelPath).toHaveValue("D:\\Projects\\funasr-demo\\model\\SenseVoiceSmall");

    await user.click(screen.getByRole("button", { name: "保存 ASR 设置" }));
    expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({
      transcription: expect.objectContaining({
        localModelPath: "D:\\Projects\\funasr-demo\\model\\SenseVoiceSmall",
      }),
    }));
  });

  it("保存 MiMo 大模型托管配置", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const saveSettings = vi.spyOn(client, "saveProviderSettings");
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    await user.click(await screen.findByRole("button", { name: "大模型" }));
    await user.selectOptions(await screen.findByLabelText("会议纪要服务商"), "xiaomi_mimo_llm");
    await user.selectOptions(screen.getByLabelText("会议纪要模型"), "mimo-v2.5-pro");
    await user.click(screen.getByRole("button", { name: "保存大模型设置" }));
    expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({
      minutes: expect.objectContaining({
        presetId: "xiaomi_mimo_llm",
        kind: "openai_compatible",
        endpoint: "https://api.xiaomimimo.com/v1/chat/completions",
        model: "mimo-v2.5-pro",
      }),
    }));

  });

  it("保存本地转写预设时使用固定模型标识且不发送密钥", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const saveSettings = vi.spyOn(client, "saveProviderSettings");
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    await screen.findByLabelText("语音转写服务商");
    await user.click(screen.getByRole("button", { name: "保存 ASR 设置" }));
    expect(saveSettings).toHaveBeenLastCalledWith(expect.objectContaining({
      transcription: expect.objectContaining({
        presetId: "local_funasr",
        kind: "local_funasr",
        endpoint: "local://model/SenseVoiceSmall",
        model: "SenseVoiceSmall",
        apiKey: "",
      }),
    }));
  });

  it("默认使用 DeepSeek 推荐模型保存参数且不回显密钥", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const saveSettings = vi.spyOn(client, "saveProviderSettings");
    render(<App client={client} />);
    await user.click(screen.getByRole("button", { name: "设置" }));

    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "大模型" }));
    await user.selectOptions(screen.getByLabelText("会议纪要服务商"), "deepseek");
    expect(screen.getByLabelText("会议纪要模型")).toHaveValue("deepseek-v4-flash");

    const sentinelSecret = "test-only-secret-value";
    await user.type(screen.getByLabelText("会议纪要 API Key"), sentinelSecret);
    await user.click(screen.getByRole("button", { name: "保存大模型设置" }));

    expect(await screen.findByText("设置已保存")).toBeInTheDocument();
    expect(screen.queryByDisplayValue(sentinelSecret)).not.toBeInTheDocument();
    expect(screen.getAllByText("密钥已配置")).toHaveLength(1);
    expect(saveSettings).toHaveBeenCalledWith(expect.objectContaining({
      transcription: expect.objectContaining({
        presetId: "local_funasr",
        endpoint: "local://model/SenseVoiceSmall",
        model: "SenseVoiceSmall",
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

    expect(await screen.findByRole("heading", { name: "开始前，请先完成处理配置" })).toBeInTheDocument();
    expect(screen.getByText("本地 ASR 语音模型")).toBeInTheDocument();
    expect(screen.getByText("纪要生成大模型接口")).toBeInTheDocument();
    expect(screen.getByText("本地模式已启用，请先在设置中检查环境")).toBeInTheDocument();
    expect(screen.getAllByText("请补充：API Key")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "选择音频或视频" })).toBeDisabled();
    expect(screen.getByLabelText("选择本地媒体文件")).toBeDisabled();
    expect(screen.getByText("配置服务后选择媒体").closest(".file-dropzone")).toHaveAttribute("aria-disabled", "true");
    expect(screen.queryByText(/Mock/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "打开服务设置" }));
    expect(await screen.findByRole("heading", { name: "设置" })).toBeInTheDocument();
  });

  it("测试连接会在对应服务区块内返回明确结果", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);

    await user.click(screen.getByRole("button", { name: "设置" }));
    const transcriptionSection = await screen.findByRole("region", { name: "本地 ASR 模型" });
    await user.click(within(transcriptionSection).getByRole("button", { name: "检查环境" }));
    expect(await within(transcriptionSection).findByText(/Windows 桌面应用中检查环境/)).toBeInTheDocument();
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
