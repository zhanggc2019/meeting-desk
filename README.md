# 听见纪要

一个小巧的 Windows 离线音视频工作台。用户选择本地 WAV、MP3、M4A、MP4 或 MOV 文件后，应用负责创建转写任务、生成结构化 AI 纪要，并在本地保存、搜索、预览和导出结果。

> 本项目只处理用户主动选择的离线文件，不采集麦克风或系统声音。

## 项目状态

当前版本为 `0.3.1`，适合桌面界面开发、配置流程验证和本地自动化测试。

- ✅ Tauri 2 桌面壳、React 界面和 SQLite 本地存储可运行。
- ✅ 内部离线测试链路已贯通：`导入媒体 → 转写 → 生成纪要 → 保存 → 展示 → Markdown 预览/导出`；正式界面不提供演示模式。
- ✅ 支持单文件和批量处理；批量任务相互独立，单项失败不影响其他文件。
- ✅ API Key 交由 Windows 凭据管理器保存，不写入前端状态、SQLite 或普通日志。
- ✅ 服务未配置完成时会禁用媒体选择，避免创建无法执行的任务。
- ✅ 支持启动时静默检查 GitHub Release；发现签名更新后由用户确认下载、安装和重启。
- ✅ GitHub Actions 可在 `main` 分支执行 Windows CI，并在 `v*` 标签上生成签名 NSIS 安装包和 `latest.json`。
- ✅ 语音转写使用仓库 `model/SenseVoiceSmall` 的本地 FunASR 推理，不上传音频且无需 ASR API Key。
- ⚠️ DeepSeek、阿里云百炼通义千问及第三方 OpenAI-compatible 纪要 Provider 已接入编排，但真实 API 互操作仍为 **BLOCKED**。当前版本不会把未验证的真实请求伪装为成功。

## 功能

- 导入 WAV、MP3、M4A 音频，以及含 AAC/ALAC 音轨的 MP4、MOV 视频。
- 在“单个文件”和“批量处理”之间显式切换。
- 批量追加文件、逐项校验、独立创建任务。
- 任务状态、取消、失败重试与重启恢复提示。
- 本地会议历史、搜索、详情查看和复制。
- 历史记录区分录音时长与总处理耗时；处理耗时不单独落库、不建立使用统计，只从任务恢复所需的既有创建/完成时间即时计算。
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
- 桌面端安全连接探测：不发送媒体、提示词或响应正文，可分类认证失败、路径错误、限流、服务端错误和超时。
- 软件自动更新：只安装由项目更新私钥签名、且来自本仓库 GitHub Release 的更新包。

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
- Python 3.12 x64（仅源码开发需要；安装包自带运行时）
- FFmpeg（处理 MP4/MOV 等容器格式时需要）

## 快速开始

安装依赖：

```powershell
pnpm install --frozen-lockfile
python -m venv .venv
.\.venv\Scripts\python.exe -m pip install -r .\src-tauri\python\requirements-local-asr.txt
```

安装包内置 Python 3.12、FunASR、ModelScope、PyTorch 和 torchaudio，不依赖系统 Python 或开发 `.venv`。模型仍独立保存在本机：下载完成后，将完整的 `SenseVoiceSmall` 文件夹放到 `%LOCALAPPDATA%\com.internal.meetingdesk\model\SenseVoiceSmall`；也可以在“服务设置 → 语音转写”的“模型路径”中直接选择已有模型目录，无需复制。

模型目录至少需要包含 `config.yaml`、`model.pt` 和 `tokens.json`。空路径会自动发现安装版默认目录或仓库根目录。`model\` 与生成的 `src-tauri\runtime\` 均被 Git 忽略；模型不进入安装包。完成后点击“检查环境”，该检查会实际加载一次模型。

启动浏览器开发模式：

```powershell
pnpm dev
```

浏览器模式使用确定性的本地测试客户端，仅用于界面开发，不会调用真实转写或大模型服务。

启动 Windows 桌面应用：

```powershell
pnpm tauri:dev
```

## 使用流程

1. 启动应用，按照首页引导打开“服务设置”。
2. 确认 `model\SenseVoiceSmall` 存在，在“服务设置”中点击“检查环境”实际加载本地模型，并配置会议纪要服务。
3. 选择“单个文件”或“批量处理”。
4. 选择一个或多个本地音频/视频，批量模式下还可以继续追加。
5. 选择纪要模板并创建任务。
6. 在任务队列查看每个文件的独立处理状态。
7. 完成后查看纪要、逐字稿和 Markdown 预览，或导出 `.md` 文件。

应用不会移动或修改源文件。它只在 Tauri 本地应用目录创建受管暂存副本，并在完成、取消或启动恢复时清理不再需要的副本。

## Provider 配置

普通用户不需要填写内置 Provider 的 Base URL：

| 用途 | 内置预设 | 模型 |
| --- | --- | --- |
| 语音转写 | 本地 FunASR | `model/SenseVoiceSmall` |
| 会议纪要 | Xiaomi MiMo 大模型 | `mimo-v2.5`、`mimo-v2.5-pro` |
| 会议纪要 | DeepSeek | `deepseek-v4-flash`、`deepseek-v4-pro` |
| 会议纪要 | 阿里云百炼（通义千问） | `qwen-plus`、`qwen-flash`、`qwen-max` |
| 会议纪要 | 第三方 OpenAI Chat Completions | 自定义完整地址与模型名，读取 `choices[0].message.content` |

语音转写固定为本地 `SenseVoiceSmall`，旧的在线 ASR 设置会自动迁移到本地预设。只有第三方 OpenAI Chat Completions 纪要预设会显示可编辑的服务地址与模型名；托管纪要预设的地址和模型白名单由 Rust 后端校验，前端不能覆盖。

API Key 的处理原则：

- 通过一次性 Tauri IPC 发送给 Rust。
- 保存到 Windows 凭据管理器。
- 保存成功后不回显。
- 不写入 Git、`.env.example`、SQLite、前端日志或会议日志。
- 密钥绑定到具体 Provider 预设；切换服务商时不会静默复用旧密钥。

本地 FunASR 适配边界：

- Rust Provider 只接收导入模块提供的受管只读文件句柄，并在应用临时目录创建生命周期受控的推理副本。
- Python 子进程固定使用 CPU、本地模型和离线环境变量；取消或超时会终止子进程。
- Python stdout/stderr 不进入普通日志；转写结果通过有大小上限的临时 JSON 返回并在 Provider 完成后删除。
- 安装版优先使用 `runtime\python\python.exe`，开发模式回退到 `.venv\Scripts\python.exe`；推理脚本和模型路径仍可通过 `.env.example` 中的进程环境变量覆盖。

`.env.example` 仅用于说明高级环境变量，不会被应用自动加载。普通用户应优先通过应用内设置完成配置。

```powershell
pnpm tauri:dev
```

模型目录不会进入 Git 或安装包。开发环境按上面的 PowerShell 命令创建 `.venv`；发布构建会执行 `pnpm runtime:prepare` 生成并校验可迁移的内置 Python 运行时。

## 测试

```powershell
pnpm typecheck
pnpm test
pnpm test:integration
pnpm runtime:check
cargo test --manifest-path src-tauri\Cargo.toml --all-targets --all-features
cargo clippy --manifest-path src-tauri\Cargo.toml --all-targets --all-features -- -D warnings
pnpm build
```

`pnpm test:integration` 使用临时生成的 WAV、内部测试 Provider、会议纪要校验器和内存 SQLite 跑通后端闭环，不调用网络，也不依赖仓库中的私人录音。内部测试 Provider 仅存在于开发测试代码，不作为正式界面功能提供。

## 构建 Windows 安装包

本地开发构建不生成自动更新签名文件：

```powershell
pnpm tauri:build
```

构建成功后会生成：

```text
src-tauri/target/release/meeting-desk.exe
src-tauri/target/release/bundle/nsis/MeetingDesk_0.3.1_x64-setup.exe
```

Windows 安装器使用英文内部产品名 `MeetingDesk`，因此新安装的默认目录、安装包文件名和卸载项不会再使用中文路径。应用窗口、界面品牌和 GitHub Release 仍显示中文名“听见纪要”。Windows 不会自动重命名已经存在的中文安装目录；已有用户若要迁移目录，应卸载旧版后安装 `0.3.1`，稳定的应用标识 `com.internal.meetingdesk` 保持不变，本地应用数据目录不受产品显示名调整影响。

这些二进制产物已被 `.gitignore` 排除。需要对外分发时，请上传到 GitHub Releases，不要直接提交到源码仓库。

## GitHub 自动打包与软件更新

- `.github/workflows/windows-ci.yml`：推送或合并到 `main` 时运行类型检查、前后端测试、Clippy 和 Windows NSIS 打包，并上传 Actions artifact。
- `.github/workflows/release.yml`：推送与应用版本一致的标签（例如 `v0.3.1`）后，创建 GitHub Release，上传 NSIS 安装包、签名文件和更新清单 `latest.json`。
- Tauri 更新端点固定为本仓库的 `releases/latest/download/latest.json`；应用不会接受缺少有效签名的更新。

> **推送 `main` 不会发布新版本。** 它只运行 Windows CI，并把安装包保存为该次 Actions Run 的 artifact。只有同时更新应用版本、提交到 `main`，再推送对应的 `v*` 标签，才会触发正式 Release 工作流。若 Actions 页面只出现 `Windows CI` 而没有 `Publish signed Windows release`，应先检查远程是否存在与当前版本一致的新标签。

> 仓库已设为 Public。2026-07-19 已从未登录环境验证：`latest.json` 返回 HTTP 200，版本为 `0.2.0`；Tauri updater 使用 `Accept: application/octet-stream` 下载 GitHub API 资产时返回 HTTP 200 和 Windows 安装包。应用不内置 GitHub Token。

发布前需在仓库 Actions secrets 中配置 `TAURI_SIGNING_PRIVATE_KEY`。私钥只保存在发布者的安全位置及 GitHub Secret 中，禁止提交到 Git；应用内只包含公钥。生成一套新密钥可运行：

```powershell
pnpm tauri signer generate -w "$env:LOCALAPPDATA\meeting-desk-release\updater.key"
```

设置 Secret 后，将三个版本号保持一致（`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`），再创建并推送标签：

```powershell
git tag -a v0.3.1 -m "听见纪要 v0.3.1"
git push origin main
git push origin v0.3.1
```

若要在本机验证签名发布构建，请仅在当前 PowerShell 进程设置私钥路径，然后运行：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY = "$env:LOCALAPPDATA\meeting-desk-release\updater.key"
pnpm tauri:build:release
Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY
```

## 项目结构

```text
frontend/src/              React 页面、组件、状态和桌面端客户端
src-tauri/src/commands/    Tauri IPC 命令与任务编排
src-tauri/src/ingest/      音视频容器/音轨校验、哈希、暂存与清理
src-tauri/src/providers/   Provider 契约、内部测试实现与 HTTP 基础设施
src-tauri/src/minutes/     Schema、模板、Prompt、校验与 Markdown
src-tauri/src/storage/     SQLite repository
shared/schemas/            MeetingMinutes JSON Schema
shared/fixtures/           匿名化的正反测试样例
docs/                      架构、API、安全、测试和 UI 文档
```

## 隐私与仓库安全

- 本地录音/视频、导出文件、SQLite、日志、环境变量、更新私钥和安装包均由 `.gitignore` 排除。
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

- 当前本地 ASR 每个任务会重新加载模型，批量任务的模型常驻复用尚未实现。
- SenseVoiceSmall 当前只输出纯文本和语种，不提供说话人标签、置信度或可靠的逐句时间戳。
- DeepSeek、阿里云百炼及第三方兼容纪要服务的真实请求与结构化响应仍未使用有效密钥完成互操作验证。
- 长转写的 map/reduce 分块总结尚未实现。
- MP3 导入校验暂不计算时长。
- 远端任务撤销需要在供应商契约验证后接入。
- MP4/MOV 当前仅接受含单条 AAC 或 ALAC 音轨的 ISO BMFF 文件；不内置 FFmpeg，不处理 AVI/MKV/WebM。
- Windows 安装包具备 Tauri 更新签名，但尚未配置商业 Authenticode 代码签名证书，首次安装可能触发 SmartScreen 提示。
- 未完成任务在应用重启后会标记为中断并要求重新选择媒体；已完成会议仍可查看。
