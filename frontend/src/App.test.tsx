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

describe("Windows 离线音频工作台", () => {
  beforeEach(() => {
    resetAppStore();
  });

  it("校验单个文件并创建独立处理任务", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);

    const file = new File(["safe mock bytes"], "示例讨论.mp3", { type: "audio/mpeg" });
    const singleFileInput = screen.getByLabelText("选择本地音频文件");
    expect(singleFileInput).not.toHaveAttribute("multiple");
    await user.upload(singleFileInput, file);

    expect((await screen.findAllByText("示例讨论.mp3")).length).toBeGreaterThan(0);
    expect(screen.getByText("可处理")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "使用 Mock 体验" }));
    await user.click(screen.getByRole("button", { name: "开始处理" }));

    expect(await screen.findByRole("heading", { name: "任务队列" })).toBeInTheDocument();
    expect((await screen.findAllByText("示例讨论.mp3")).length).toBeGreaterThanOrEqual(2);
  });

  it("对空文件显示校验错误且不允许提交", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);

    const emptyFile = new File([], "空文件.wav", { type: "audio/wav" });
    await user.upload(screen.getByLabelText("选择本地音频文件"), emptyFile);

    expect(await screen.findByText("文件为空")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "使用 Mock 体验" }));
    expect(screen.getByRole("button", { name: "开始处理" })).toBeDisabled();
  });

  it("显式切换批量模式并为多个文件创建独立任务", async () => {
    const user = userEvent.setup();
    const client = createMockDesktopClient();
    const createTasks = vi.spyOn(client, "createProcessingTasks");
    render(<App client={client} />);

    await user.click(screen.getByRole("button", { name: "批量处理" }));
    expect(screen.getByText(/单个文件失败不会影响本批次的其他文件/)).toBeInTheDocument();
    expect(screen.getByLabelText("批量选择本地音频文件")).toHaveAttribute("multiple");

    const files = [
      new File(["first safe mock bytes"], "课程上半场.mp3", { type: "audio/mpeg" }),
      new File(["second safe mock bytes"], "课程下半场.wav", { type: "audio/wav" }),
      new File([], "空录音.wav", { type: "audio/wav" }),
    ];
    await user.upload(screen.getByLabelText("批量选择本地音频文件"), files);

    expect(await screen.findByRole("heading", { name: "本批次文件" })).toBeInTheDocument();
    expect(screen.getByText("3 个文件")).toBeInTheDocument();
    expect(screen.getByText("2 个可处理")).toBeInTheDocument();
    expect(screen.getByText("1 个校验失败")).toBeInTheDocument();
    expect(screen.getByText("文件为空")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "继续添加" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "单个文件" })).toBeDisabled();

    await user.click(screen.getByRole("button", { name: "使用 Mock 体验" }));
    await user.click(screen.getByRole("button", { name: "创建 2 个处理任务" }));

    expect(await screen.findByRole("heading", { name: "任务队列" })).toBeInTheDocument();
    expect(createTasks).toHaveBeenCalledOnce();
    expect(createTasks.mock.calls[0]?.[0]).toHaveLength(2);
    expect((await screen.findAllByText("课程上半场.mp3")).length).toBeGreaterThanOrEqual(1);
    expect((await screen.findAllByText("课程下半场.wav")).length).toBeGreaterThanOrEqual(1);
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

    await user.click(await screen.findByRole("button", { name: /^产品交付节奏讨论/ }));
    expect(await screen.findByRole("heading", { name: "产品交付节奏讨论" })).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "会议摘要" })).toBeInTheDocument();

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

    await user.selectOptions(screen.getByLabelText("会议纪要服务商"), "deepseek");
    expect(screen.queryByLabelText("会议纪要服务地址")).not.toBeInTheDocument();
    expect(screen.getByLabelText("会议纪要模型")).toHaveValue("deepseek-v4-flash");
    expect(within(screen.getByLabelText("会议纪要模型")).getByRole("option", { name: "deepseek-v4-pro" })).toBeInTheDocument();

    await user.selectOptions(screen.getByLabelText("会议纪要服务商"), "custom_openai_compatible");
    expect(screen.getByLabelText("会议纪要服务地址")).toBeInTheDocument();
    expect(screen.getByLabelText("会议纪要模型")).toHaveAttribute("placeholder", "输入已验证的模型名");
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
    expect(screen.getAllByText("当前为 Mock 演示模式")).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "打开服务设置" }));
    expect(await screen.findByRole("heading", { name: "服务设置" })).toBeInTheDocument();
  });

  it("明确启用 Mock 后仍持续标明演示模式", async () => {
    const user = userEvent.setup();
    render(<App client={createMockDesktopClient()} />);

    await user.click(await screen.findByRole("button", { name: "使用 Mock 体验" }));
    expect(screen.getByRole("heading", { name: "当前使用 Mock 体验" })).toBeInTheDocument();
    expect(screen.getByText(/不会调用 FunASR 或大模型/)).toBeInTheDocument();
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
