# Windows 离线音视频转写与 AI 会议纪要工具：技术架构

> 状态：Phase 0 技术决策基线
> 日期：2026-07-17
> 适用范围：Windows 11 x64、单文件与批量离线音视频导入
> 本文不定义任何真实云厂商的私有 HTTP 字段；真实接口必须由后续阶段以最小样本验证。

## 1. 结论先行

桌面端明确选择 **Tauri 2 + React + TypeScript + Vite + Rust + SQLite**。

应用只处理用户通过 Windows 文件选择器选中的已有离线音频或视频，不创建音频流。就本地文件处理而言，Rust core 只负责文件选择之后的安全校验、受管 staging、流式 hash 和 Provider capability preflight；云端请求、持久化和导出也留在受信任后端，以避免 Key 和任意文件访问进入 WebView。WebView 只负责交互与展示。

导入入口支持单个或批量选择 WAV、MP3、M4A、MP4、MOV。MP4/MOV 在 Rust core 中解析 ISO BMFF 结构，并要求存在一条 AAC (`mp4a`) 或 ALAC 音轨；视频画面不在本地解码。是否能提交还必须以当前 `TranscriptionProvider.capabilities()` 返回或已验证配置的容器、媒体类型、编码、大小和时长约束为准。

MVP 不默认转码、不默认切片，不捆绑或调用 FFmpeg。某个供应商不接受源文件时，应用应给出可操作的 `unsupported_media` 或 limit 错误，而不是静默改变文件内容。后续是否加入转换能力必须由真实 Provider 证据和产品决策驱动。

## 2. Phase 0 仓库与环境证据

Phase 0 初始检查结果：

| 项目 | 实际结果 |
| --- | --- |
| 初始内容 | 根目录只有 `AGENTS.md` 与一份本地 MP3 测试资产，无应用骨架 |
| Git | Phase 0 入口时不是 Git repository；随后已初始化并可执行 `git diff` 审查 |
| Windows | 64 位 Windows，系统 API 返回 build `26200` |
| Node/npm/pnpm | Node `v24.9.0`、npm `11.12.1`、pnpm `10.33.0` |
| Rust | `rustc 1.92.0`、`cargo 1.92.0`、`stable-x86_64-pc-windows-msvc` |
| Windows 构建工具 | Visual Studio 2022 Build Tools 已安装；后续 Tauri dev、release 和 NSIS 构建均已验证 |
| WebView2 | 运行时已存在 |
| 测试资产 | MP3、16 kHz、单声道、约 32 分 29 秒、约 31 MB；只检查了技术元数据，没有读取正文 |

Phase 0 没有创建应用代码，没有调用真实 Provider，也没有声称转写、构建或打包已经通过。

## 3. Tauri 2 选型

Tauri 2 满足当前范围：

- 应用复用系统 WebView2，适合“小巧”的 Windows 内部工具。Tauri Windows installer 可按联网 bootstrapper、离线安装器等方式处理 WebView2；具体 installer 大小必须在真实构建后记录。[Tauri：Windows Installer](https://v2.tauri.app/distribute/windows-installer/)
- Rust core 与 WebView 分离，文件、密钥、网络和数据库可以留在受信任后端。[Tauri：Process Model](https://v2.tauri.app/concept/process-model/)
- Tauri 2 capabilities 可对白名单窗口限制命令面，并默认阻止远程来源访问应用 API。[Tauri：Capabilities](https://v2.tauri.app/security/capabilities/)
- 当前业务不需要额外桌面运行时或媒体管线；Tauri 2 作为最终桌面壳，不再保留另一套壳的并行实现路径。

已实测 Tauri release build、NSIS 生成、updater `.sig` 生成和 release exe 启动。尚未实测 NSIS 安装/卸载、旧版到新版的真实升级、企业代理、自签 CA、WebView2 更新策略和 Authenticode 代码签名；这些事项必须在正式企业分发前验证。

### 3.1 自动更新与发布信任链

- 桌面端使用 Tauri 2 updater；启动后静默请求 GitHub Releases 的 `latest.json`，只有发现更高版本时才显示更新提示。
- `latest.json`、NSIS 安装包和 `.sig` 由标签触发的 GitHub Actions 生成。更新私钥仅存在于 GitHub Actions Secret 和发布者安全目录，应用包只嵌入公钥。
- 更新包必须通过 Tauri 签名验证，签名校验不可在客户端关闭；下载或安装失败只显示安全错误，不输出响应正文或 URL 参数。
- 普通 `main` CI 使用覆盖配置关闭 updater artifact 生成，因此不需要向非发布工作流暴露签名私钥。
- 该更新签名用于保证更新来源和完整性，不等同于 Windows Authenticode。企业分发仍应配置独立代码签名证书并验证 SmartScreen 策略。
- GitHub Release updater 端点必须对未登录客户端公开。Private 仓库会让桌面客户端得到 404；不得通过在应用中嵌入仓库 Token 绕过。启用前必须改为 Public 或迁移到无需登录的签名资产分发端点。

## 4. 总体架构

```text
┌──────────────────────── React / TypeScript UI ────────────────────────┐
│ 媒体选择 │ 批量列表 │ 处理状态 │ 历史搜索 │ 纪要详情 │ 设置 │ 导出 │ 更新 │
└──────────────────── typed commands / safe events ─────────────────────┘
                                  │
┌──────────────────────────── Tauri Rust Core ──────────────────────────┐
│ File Ingest                                                          │
│   validate -> stage -> hash -> provider capability preflight          │
│                                                                      │
│ Task Orchestrator                                                    │
│   upload -> transcribe -> summarize -> validate -> persist            │
│                                                                      │
│ TranscriptionProvider / MinutesProvider / Mock Providers             │
│ SQLite Repository / Markdown Export / Credential Store / Updater     │
└──────────────────────────────────────────────────────────────────────┘
       │ app-local-data                │ HTTPS              │ vault
  staged files + SQLite           cloud providers      provider secrets
```

### 4.1 信任边界

- **WebView 是受限展示层。** 它不能直接读取密钥、任意路径或 SQLite，不持久化 Key，也不自行发送带认证头的请求。
- **Rust core 是受信任层。** 它只接受桌面文件选择器产生的引用，重新校验文件后才生成受管 artifact。
- **源文件属于用户。** 应用不修改、移动或删除原文件。staging 失败不能破坏原文件。
- **云端响应是不受信任输入。** HTTP body 和模型输出必须限制大小、解析并经过 Schema 校验；返回文本不能作为 HTML 执行。
- **普通日志是低敏感诊断通道。** 只允许 task/artifact ID、阶段、耗时、安全错误码和 HTTP status；禁止 Key、认证头、原始文件名、完整 hash、文件内容、完整转写和完整纪要。

## 5. 技术栈与依赖边界

| 层 | 推荐 | 约束 |
| --- | --- | --- |
| 桌面壳 | Tauri 2 | Windows 11 x64 首发；使用项目本地 CLI |
| 前端 | React + TypeScript + Vite | strict TypeScript；不直接打开任意文件或持有秘密值 |
| UI | Tailwind CSS + 少量源码型 primitives | 不引入重量级全套组件框架 |
| 路由 | React Router | 路由只表示 UI，不成为任务事实源 |
| 前端状态 | Zustand | 只保存临时交互状态；业务事实来自 Rust/SQLite |
| 文件导入 | Rust ingest service | 流式读取；格式/大小校验；staging；SHA-256；不默认转码 |
| 异步与取消 | Tokio + CancellationToken | 上传、转写轮询和总结均可取消 |
| HTTP | 单例 `reqwest::Client` | 统一超时、错误分类、代理策略、响应上限和 redaction |
| 持久化 | SQLite + rusqlite（bundled）+ 幂等内联迁移 | 只由 Rust repository 访问；事务化保存任务和结果，减少 MVP 依赖与运行时复杂度 |
| 密钥 | Windows Credential Manager 的受控 adapter | SQLite 只保存 credential reference/是否已配置 |
| 业务结构 | JSON Schema + TypeScript/Rust 类型 + fixtures | 模型输出必须校验并带 schema version |
| 测试 | Vitest、Testing Library、Rust unit/integration、mock provider | 无真实 Key 时可完成核心闭环 |
| 包管理 | pnpm + Cargo | `packageManager` 与 lockfiles 在骨架建立后固定 |

客户端不得依赖开发机已安装的媒体工具，也不得在发布包中捆绑 FFmpeg。文件格式解析优先使用受维护的库并锁定版本；依赖选择由 Phase 1 构建与测试结果确定。

## 6. 文件导入与 artifact 生命周期

### 6.1 统一流程

```text
SelectedFileRef
  -> validate path and regular-file status
  -> sniff container/media metadata
  -> check non-empty and configured local limits
  -> copy to <artifact-id>.part in app-local-data
  -> calculate SHA-256 while streaming
  -> flush + validate staged copy
  -> rename to managed artifact
  -> provider capability preflight
  -> create processing task
```

约束：

- 只接受用户在当前交互中选中的文件引用，不提供前端任意路径读取命令。
- 支持 WAV、MP3、M4A、MP4、MOV 的入口筛选，但必须同时验证文件头、容器和音轨，不只看后缀。
- staging 采用分块 I/O，不把大文件整体加载到内存；临时文件只位于 app-local-data。
- hash 针对 staged copy 计算，用于完整性、去重提示和幂等辅助；不能将完整 hash 写入普通日志。
- staging 前后检查源文件大小和修改时间。若复制过程中源文件变化，返回 `source_file_changed`，不提交不一致副本。
- 成功完成后只管理 staged copy；删除会议不得删除用户原文件。
- 应用异常退出可能留下 `.part`，下次启动只允许清理或重新导入，不能把它提升为可上传 artifact。

### 6.2 Provider capability preflight

中立能力结构至少表达：

```text
TranscriptionCapabilities {
  acceptedExtensions
  acceptedMediaTypes
  acceptedContainers?
  acceptedCodecs?
  maxBytes?
  maxDurationMs?
  uploadMode
  supportsRemoteCancel
  supportsSpeakerLabels
  supportsTimestamps
}
```

preflight 规则：

- 只根据 adapter 明确声明或真实接口验证后的能力拒绝请求，不猜测缺失限制。
- `unknown` 与“不支持”是不同状态；未知约束记录在 contract 文档，并由真实响应进一步分类。
- 大小、时长、容器、媒体类型或编码明确不兼容时，在上传前返回安全的 `unsupported_media`、`file_too_large` 或 `duration_limit_exceeded`。
- 不为满足能力表而自动转码、压缩或切片。

## 7. 任务编排与取消

```text
selected
  -> validating
  -> staging
  -> preflight
  -> queued
  -> uploading
  -> transcribing
  -> validating_transcript
  -> summarizing
  -> validating_minutes
  -> saving
  -> completed

uploading / transcribing / summarizing
  -> cancel_requested
  -> cancelled

可安全重试阶段 -> retry_wait -> 对应阶段
任一阶段 -> failed
应用重启发现活动任务 -> interrupted
```

规则：

- 进度只展示真实阶段。Provider 没有百分比时不伪造精确进度。
- 每次状态变化和 attempt 先在 SQLite 事务中保存，再发不含正文的状态事件。
- 上传中的取消必须中止本地请求体传输；转写和总结阶段必须中止请求或轮询。
- 若远端接口没有撤销作业能力，adapter 必须将 `supportsRemoteCancel=false` 暴露给上层；本地进入 `cancelled` 不得声称远端作业已被撤销。
- `completed` 只在 transcript、schema-valid minutes 和 meeting record 均持久化后出现。
- 文件 hash、task id 和数据库唯一约束共同防止重复点击产生多个活动任务；是否复用历史结果由用户确认，不能静默合并。

### 7.1 错误分类

| 类别 | 示例错误码 | 默认策略 |
| --- | --- | --- |
| 文件 | `file_not_found`, `empty_file`, `invalid_media`, `source_file_changed` | 不自动重试 |
| 能力 | `unsupported_media`, `file_too_large`, `duration_limit_exceeded` | 上传前失败，提示更换文件或 Provider |
| 配置 | `provider_not_configured`, `invalid_endpoint` | 不重试，进入设置修复 |
| 认证 | `http_401`, `http_403` | 不重试，不回显秘密值 |
| 限流 | `http_429` | 尊重 `Retry-After`，有限退避 |
| 临时网络 | `network_unavailable`, `timeout`, `http_5xx` | 仅对可安全重放操作有限退避 |
| 取消 | `cancelled` | 不重试，清理未完成 staging/上传状态 |
| 响应 | `invalid_provider_response`, `schema_validation_failed` | 保存安全诊断元数据，有限策略处理 |
| 存储 | `database_error`, `disk_full`, `export_failed` | 不掩盖，不产生假完成状态 |

## 8. Provider-agnostic 云端边界

### 8.1 中立接口

```text
TranscriptionProvider
  capabilities() -> TranscriptionCapabilities
  transcribe(ManagedAudioArtifact, TranscriptionOptions, CancellationToken)
    -> Transcript

MinutesProvider
  capabilities() -> MinutesCapabilities
  generate(Transcript, MinutesTemplate, JSONSchema, CancellationToken)
    -> MeetingMinutesEnvelope
```

`ManagedAudioArtifact` 只暴露 artifact ID、受控读取句柄或后端内部路径、已验证媒体元数据、字节数和 hash；原始用户路径不得进入 Provider adapter。

`Transcript` 支持纯文本以及可选 segments、speaker label、timestamp、confidence。Provider 缺少可选信息时保持缺失，不伪造。

`MeetingMinutesEnvelope` 必须带 `schemaVersion`、结构化 minutes、校验结果和安全模型元信息；不得附带完整 prompt 或请求正文。

### 8.2 OpenAI-compatible 的含义

“OpenAI-compatible”只表示可实现通用 adapter，不表示所有供应商的路径、multipart 字段、异步作业、响应 JSON、媒体限制或 structured output 完全相同。因此：

- 配置分别保存 ASR/LLM endpoint、model、timeout、retry、并发和 credential reference。
- Provider-specific 差异只能留在独立 adapter/codec，不得渗透到 UI、任务编排或数据库 schema。
- 每个 codec 都要有 contract fixtures；真实字段只在 `docs/api-contract.md` 中记录经验证的事实。
- 未知时只实现中立接口和 mock，不编造真实请求或响应。

### 8.3 Mock-first

- mock ASR 接收 artifact 安全元数据，返回固定、非敏感 fixture transcript。
- mock minutes 返回符合 JSON Schema 的结构化 fixture。
- mock 可注入 delay、timeout、401、429、500、malformed JSON、empty transcript 和取消。
- mock 调用记录只包含 request ID、artifact ID、阶段和安全元数据。

## 9. 会议纪要与持久化

结构化纪要至少包含：标题、会议时间、参会人、摘要、主要议题、关键结论、决策事项、待办事项、风险和问题。参会人未知时用空数组，speaker label 不能被解释为真实姓名。

建议 SQLite 实体：

- `meetings`：会议元数据、模板、状态和时间戳。
- `audio_artifacts`：内部 ID、受控路径、媒体元数据、字节数、hash、staging/cleanup 状态。
- `transcripts`：全文、segments JSON、schema version。
- `minutes`：结构化 JSON、schema version、渲染版本。
- `tasks` / `task_attempts`：阶段、安全错误码、attempt 和时间戳。
- `app_settings`：非秘密配置和 credential reference。

数据库与 staged files 位于 app-local-data，不写仓库。导出只在用户显式选择的位置生成 Markdown。删除会议应删除数据库记录和受管副本，但永远不删除用户原文件。

## 10. Tauri IPC 与跨 Agent 契约

### 10.1 建议 DTO

```text
SelectedFileRef { selectionToken, displayName }
ImportedAudioFile { artifactId, mediaType, container?, codec?, byteLength,
                    durationMs?, sha256, status }
ImportValidation { accepted, error?, detectedMetadata, capabilityCheck }
ProcessingTask { id, meetingId?, artifactId, status, stage, attempt,
                 cancelCapability, error?, timestamps }
TaskError { code, category, retryable, safeMessage, httpStatus?, retryAfterMs? }
Transcript { schemaVersion, text, language?, segments[] }
MeetingMinutes { schemaVersion, title, time, participants[], summary, topics[],
                 conclusions[], decisions[], actionItems[], risks[] }
```

UI 不依赖绝对路径。`TaskError.safeMessage` 是唯一可直接展示的错误信息，底层 exception 和 Provider body 不经 IPC 发送。

### 10.2 建议命令面

| 命令组 | 命令 |
| --- | --- |
| ingest | `import_selected_files`, `get_import_validation`, `remove_staged_artifact` |
| tasks | `create_processing_tasks`, `cancel_task`, `retry_task`, `get_task`, `list_tasks` |
| meetings | `list_meetings`, `search_meetings`, `get_meeting_detail`, `delete_meeting`, `export_meeting_markdown` |
| settings | `get_public_settings`, `save_provider_settings`, `test_provider_connection`, `delete_provider_secret` |

`get_public_settings` 只能返回 `secretConfigured` 和非秘密配置，不能返回 Key。状态事件只发送 task/artifact ID、阶段、时间和安全错误。

## 11. 推荐目录与 Agent 所有权

```text
funasr-demo/
├─ AGENTS.md
├─ README.md
├─ package.json
├─ pnpm-lock.yaml
├─ docs/
│  ├─ architecture.md          # Agent 1
│  ├─ mvp.md                   # Agent 1
│  ├─ api-contract.md          # Agent 4
│  ├─ security-review.md       # Agent 6
│  └─ test-plan.md             # Agent 6
├─ frontend/
│  └─ src/                     # Agent 2
├─ src-tauri/
│  ├─ capabilities/
│  └─ src/
│     ├─ ingest/               # Agent 3：文件校验、staging、hash、preflight
│     ├─ providers/            # Agent 4
│     ├─ minutes/              # Agent 5
│     ├─ persistence/          # Lead 指派单一所有者
│     ├─ tasks/ config/ security/
│     ├─ commands/             # Lead 集成
│     └─ lib.rs / main.rs      # Lead
├─ shared/
│  ├─ schemas/                 # Agent 5
│  └─ fixtures/                # 按 providers/minutes/ingest 分目录
└─ tests/
   ├─ integration/
   └─ e2e/
```

| 角色 | 独占写入范围 | 交付接口 |
| --- | --- | --- |
| Agent 1 | `docs/architecture.md`, `docs/mvp.md` | 架构与验收基线 |
| Agent 2 | `frontend/src/**` | typed desktop client、导入与任务 UI |
| Agent 3 | `src-tauri/src/ingest/**`, `shared/fixtures/ingest/**` | `FileIngestService`、格式校验、staging、hash、preflight |
| Agent 4 | `src-tauri/src/providers/**`, `docs/api-contract.md`, provider fixtures | provider traits、adapter、mock、错误映射 |
| Agent 5 | `src-tauri/src/minutes/**`, `shared/schemas/**`, minutes fixtures | JSON Schema、prompt、parser/validator |
| Agent 6 | `docs/security-review.md`, `docs/test-plan.md` | 审查与测试矩阵 |
| Lead | manifests、入口、commands、共享 DTO、持久化归属和集成 | 可运行骨架与端到端验证 |

子 Agent 不并发修改 manifest、`lib.rs`、共享 index/barrel 或其他角色目录。现有项目级所有权若与本表不一致，由 Lead 在进入下一阶段前统一调整。

## 12. 配置与安全原则

配置项至少包括：

```text
asr.providerType / endpoint / model / connectTimeoutMs / requestTimeoutMs
asr.maxRetries / maxConcurrent / verifiedCapabilities
minutes.providerType / endpoint / model / connectTimeoutMs / requestTimeoutMs
minutes.maxRetries / templateId
ingest.localMaxBytes / stagingRetention
mock.scenario
```

- 开发期 Key 只能由环境变量注入；`.env.example` 仅含空值或说明，`.env` 必须忽略。
- 生产 Key 保存到 Windows Credential Manager。前端提交后只能覆盖或删除，不能读回。
- endpoint 必须校验 URL。生产默认拒绝明文 HTTP；仅显式开发模式可访问本机 mock server。
- HTTP trace、panic hook、配置 dump 和错误链必须字段级 redaction。
- 源文件名可能包含会议主题或人员信息，不得出现在普通日志、遥测或测试快照中。

## 13. 架构级验收门槛

在宣布 MVP 完成前必须有真实证据证明：

1. Tauri Windows 应用可启动、构建并完成至少一种 installer 构建。
2. 单文件和混合批量选择能完成校验、staging、流式 hash 和 Provider capability preflight。
3. WAV、MP3、M4A、MP4、MOV 的入口行为有 fixtures；是否接受由内部测试/真实 capabilities 决定，扩展名伪装、空文件、损坏容器和无音轨视频被拒绝。
4. 使用仓库现有真实 MP3 或另一份非敏感文件跑通 mock 全链路：`import -> transcript -> schema-valid minutes -> SQLite -> UI -> Markdown`。
5. 上传、转写和总结三个阶段的取消均有验证；远端不支持撤销时 UI 语义诚实。
6. 401、429、500、timeout、网络中断、重复提交、批量部分失败和重启恢复有实际测试结果。
7. 应用未捆绑或调用 FFmpeg，且未默认转换、压缩或切片用户文件。
8. 类型检查、单元测试、集成测试、前端构建、Tauri build 和打包均实际执行；失败项标记 BLOCKED。
9. 普通日志和 UI 无 Key、认证头、原始文件名、完整 hash、转写正文或模型请求/响应正文。

产品验收条件见 [MVP 定义](./mvp.md)。

## 14. 主要风险

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| M4A 容器内部编码差异 | 后缀正确但 Provider 拒绝 | 文件头/容器解析 + capabilities preflight；不静默转换 |
| Provider 限制未知 | 上传后才失败 | 将未知与不支持分开；Phase 4 记录真实限制和响应 |
| 大文件内存或磁盘压力 | UI 卡顿、staging 失败 | 分块 I/O、受管临时文件、启动前空间检查、可配置本地上限 |
| 源文件复制期间变化 | hash 与用户所见不一致 | 前后 fingerprint 检查；不一致即失败 |
| 批量并发过高 | 限流和带宽竞争 | 低并发队列、Provider 级并发限制、429 退避 |
| 远端不支持撤销 | 本地取消后远端仍可能执行 | capability 明示并在 UI 说明，不伪造远端状态 |
| 文件名/正文泄露 | 企业隐私事件 | artifact ID 日志、受控 staging、后端网络、redaction 测试 |
| 无默认转换能力 | 某些文件不能提交 | 明确错误；只在真实需求证明后增加独立可选能力 |
| Phase 0 入口没有 Git metadata（已关闭） | 当时无法进行 diff 审查 | 已初始化 Git、建立忽略规则，并安排独立 Reviewer 直接审查 |

## 15. 决策记录

- **ADR-001：明确选择 Tauri 2。** 以系统 WebView2、Rust 后端和 capabilities 构建小型 Windows 工具，不维护第二套桌面壳。
- **ADR-002：只导入已有离线音频。** 桌面端没有实时音频输入链路。
- **ADR-003：Rust 负责导入后端边界。** 校验、staging、流式 hash 和 capability preflight 不放在 WebView。
- **ADR-004：源文件原样提交。** MVP 不默认转码、压缩或切片，也不捆绑 FFmpeg。
- **ADR-005：Mock-first、adapter-first。** 未知真实字段时只实现中立 contract 和 mock。
- **ADR-006：SQLite 是业务事实源。** 前端 store 不是持久任务事实源。
- **ADR-007：秘密值不进入前端持久化和 SQLite。** 生产 Key 使用 Windows Credential Manager。
