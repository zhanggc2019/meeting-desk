# 听见纪要

一个小巧的 Windows 离线音频工作台。用户选择本地 WAV、MP3 或 M4A 录音后，应用负责创建转写任务、生成结构化 AI 纪要，并在本地保存、搜索、预览和导出结果。

> 本项目只处理用户主动选择的离线文件，不采集麦克风或系统声音。

## 项目状态

当前版本为 `0.1.0`，适合开发、界面演示和 Mock 全流程验证。

- ✅ Tauri 2 桌面壳、React 界面和 SQLite 本地存储可运行。
- ✅ Mock 流程已贯通：`导入音频 → 转写 → 生成纪要 → 保存 → 展示 → Markdown 预览/导出`。
- ✅ 支持单文件和批量处理；批量任务相互独立，单项失败不影响其他文件。
- ✅ API Key 交由 Windows 凭据管理器保存，不写入前端状态、SQLite 或普通日志。
- ⚠️ 阿里云百炼 Fun-ASR 与 DeepSeek 的预设界面已完成，但真实 API 互操作仍为 **BLOCKED**。当前版本不会把未验证的真实请求伪装为成功。

## 功能

- 导入 WAV、MP3、M4A 离线录音。
- 在“单个文件”和“批量处理”之间显式切换。
- 批量追加文件、逐项校验、独立创建任务。
- 任务状态、取消、失败重试与重启恢复提示。
- 本地会议历史、搜索、详情查看和复制。
- 完整逐字稿与结构化会议纪要。
- Markdown 应用内渲染预览与 `.md` 文件导出。
- 多种纪要模板：
  - 标准会议纪要
  - 项目周会
  - 客户沟通
  - 课程总结
  - 课题研究
  - 学术讲座
  - 人物专访
  - 深度访谈
  - 商业计划书
  - 文章大纲
  - 自适应模板
- Provider 预设、模型下拉、超时与重试配置。

## 技术栈

| 层级 | 技术 |
| --- | --- |
| Windows 桌面端 | Tauri 2、Rust |
| 前端 | React 19、TypeScript、Vite |
| 状态管理 | Zustand |
| 数据校验 | Zod、JSON Schema |
| 本地数据 | SQLite（rusqlite） |
| Markdown | react-markdown、remark-gfm |
| 测试 | Vitest、Testing Library、Rust tests |
| 包管理 | pnpm |

## 环境要求

推荐使用本项目已验证过的 Windows 开发环境：

- Windows 11 x64
- Node.js 24
- pnpm 10.33.0
- Rust stable，目标工具链 `x86_64-pc-windows-msvc`
- Visual Studio 2022 C++ Build Tools
- Microsoft Edge WebView2 Runtime

## 快速开始

安装依赖：

```powershell
pnpm install --frozen-lockfile
```

启动浏览器开发模式：

```powershell
pnpm dev
```

浏览器模式使用确定性前端 Mock，不会调用真实转写或大模型服务。

启动 Windows 桌面应用：

```powershell
pnpm tauri:dev
```

## 使用流程

1. 启动应用，按照首页引导打开“服务设置”。
2. 开发或演示阶段可选择 Mock；真实服务未验证前不会发起真实处理任务。
3. 选择“单个文件”或“批量处理”。
4. 选择一个或多个本地录音，批量模式下还可以继续追加。
5. 选择纪要模板并创建任务。
6. 在任务队列查看每个文件的独立处理状态。
7. 完成后查看纪要、逐字稿和 Markdown 预览，或导出 `.md` 文件。

应用不会移动或修改源文件。它只在 Tauri 本地应用目录创建受管暂存副本，并在完成、取消或启动恢复时清理不再需要的副本。

## Provider 配置

普通用户不需要填写内置 Provider 的 Base URL：

| 用途 | 内置预设 | 模型 |
| --- | --- | --- |
| 语音转写 | 阿里云百炼 Fun-ASR（中国内地 / 国际） | `fun-asr`、`fun-asr-mtl` |
| 会议纪要 | DeepSeek | `deepseek-v4-flash`、`deepseek-v4-pro` |
| 开发验证 | Mock | 内置固定结果 |

只有“自建 / 自定义（高级）”会显示可编辑的服务地址与模型名。托管预设的地址和模型白名单由 Rust 后端校验，前端不能覆盖。

API Key 的处理原则：

- 通过一次性 Tauri IPC 发送给 Rust。
- 保存到 Windows 凭据管理器。
- 保存成功后不回显。
- 不写入 Git、`.env.example`、SQLite、前端日志或会议日志。
- 密钥绑定到具体 Provider 预设；切换服务商时不会静默复用旧密钥。

`.env.example` 仅用于说明高级环境变量，不会被应用自动加载。PowerShell Mock 启动示例：

```powershell
$env:MEETING_DESK_ASR_PRESET_ID = "mock"
$env:MEETING_DESK_ASR_PROVIDER_KIND = "mock"
$env:MEETING_DESK_LLM_PRESET_ID = "mock"
$env:MEETING_DESK_LLM_PROVIDER_KIND = "mock"
pnpm tauri:dev
```

真实 Fun-ASR 仍需要实现供应商专用的“文件上传 → 异步提交 → 轮询 → 下载并归一化结果”适配器；配置预设不代表该链路已经完成。详细边界见 [Provider API 契约](docs/api-contract.md)。

## 测试

```powershell
pnpm typecheck
pnpm test
pnpm test:integration
cargo test --manifest-path src-tauri\Cargo.toml --all-targets --all-features
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets --all-features -- -D warnings
pnpm build
```

`pnpm test:integration` 使用临时生成的 WAV、Rust Mock Provider、会议纪要校验器和内存 SQLite 跑通后端闭环，不调用网络，也不依赖仓库中的私人录音。

## 构建 Windows 安装包

```powershell
pnpm tauri:build
```

构建成功后会生成：

```text
src-tauri/target/release/meeting-desk.exe
src-tauri/target/release/bundle/nsis/听见纪要_0.1.0_x64-setup.exe
```

这些二进制产物已被 `.gitignore` 排除。需要对外分发时，请上传到 GitHub Releases，不要直接提交到源码仓库。

## 项目结构

```text
frontend/src/              React 页面、组件、状态和桌面端客户端
src-tauri/src/commands/    Tauri IPC 命令与任务编排
src-tauri/src/ingest/      音频校验、哈希、暂存与清理
src-tauri/src/providers/   Provider 契约、Mock 与 HTTP 基础设施
src-tauri/src/minutes/     Schema、模板、Prompt、校验与 Markdown
src-tauri/src/storage/     SQLite repository
shared/schemas/            MeetingMinutes JSON Schema
shared/fixtures/           匿名化的正反测试样例
docs/                      架构、API、安全、测试和 UI 文档
```

## 隐私与仓库安全

- 本地录音、导出文件、SQLite、日志、环境变量和安装包均由 `.gitignore` 排除。
- 完整逐字稿只进入本地会议记录和用户主动导出的 Markdown，不写普通运行日志。
- 删除会议只删除应用记录，不删除用户原始录音。
- 测试使用生成数据或匿名 fixture；根目录私人录音不会上传到 GitHub。
- 提交前建议运行 `git status --short --ignored`，确认没有新增的本地隐私文件进入跟踪列表。

## 文档

- [技术架构](docs/architecture.md)
- [MVP 范围](docs/mvp.md)
- [Provider API 契约](docs/api-contract.md)
- [会议纪要契约](docs/minutes-contract.md)
- [安全审查](docs/security-review.md)
- [测试计划](docs/test-plan.md)
- [UI 规范](docs/ui-spec.md)

## 已知限制

- 真实 Fun-ASR 音频上传、异步任务轮询和响应归一化尚未实现。
- DeepSeek 真实请求与结构化响应仍未使用有效密钥完成互操作验证。
- 长转写的 map/reduce 分块总结尚未实现。
- MP3 导入校验暂不计算时长。
- 远端任务撤销需要在供应商契约验证后接入。
- 未完成任务在应用重启后会标记为中断并要求重新选择音频；已完成会议仍可查看。
