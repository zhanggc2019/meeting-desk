# MVP 定义与验收标准

> 状态：Phase 0 产品范围基线
> 日期：2026-07-17
> 技术实现与模块边界见 [技术架构](./architecture.md)。本文描述“做什么、怎样算完成”，不表示能力已经实现。

## 1. MVP 目标

为企业内部 Windows 用户提供一个小巧的桌面工具：选择一个或批量选择已有的离线音频/视频文件，经可配置的云端 ASR 获得完整转写，再由可配置的大模型生成结构化会议纪要，最后在本地保存、搜索、复制、预览并导出 Markdown。

```text
选择 WAV / MP3 / M4A / MP4 / MOV
  -> 校验 / staging / hash / capability preflight
  -> 上传 / 转写 / 总结
  -> 本地保存 / 详情展示 / 搜索 / 复制 / Markdown 导出
```

支持的入口格式是 WAV、MP3、M4A、MP4、MOV；视频必须包含 AAC 或 ALAC 音轨。某个文件能否提交取决于所选 ASR Provider 的真实 capabilities。MVP 不默认转码，不捆绑或调用 FFmpeg。

## 2. 目标用户与核心场景

- 企业内部员工整理已有的会议、访谈、培训或项目沟通音频/视频文件。
- 用户自行提供 ASR 与大模型配置，应用不绑定单一供应商。
- 用户希望批量处理多个文件，并能查看每个任务的独立状态、失败原因、取消和重试结果。
- 用户希望文件、转写和纪要只存在于本机及其主动配置的云端处理链路中。

## 3. MVP 范围

### 3.1 文件导入

- 使用 Windows 文件选择器选择一个或多个已有文件。
- 入口筛选 WAV、MP3、M4A、MP4、MOV，同时在 Rust core 重新检查普通文件状态、非空、文件头/容器、音轨、大小和已知媒体元数据。
- 将通过本地校验的文件流式复制到 app-local-data staging，不修改、移动或删除用户原文件。
- staging 过程中计算 SHA-256，并检查源文件是否在复制期间变化。
- 根据 `TranscriptionProvider.capabilities()` 对扩展名、媒体类型、容器、编码、大小和时长做 preflight。
- capabilities 未声明的限制保持 `unknown`，不凭空假设；明确不兼容时在上传前失败。
- 单个文件失败不影响同一批次中的其他文件。
- 不默认转码、压缩或切片，不依赖外部媒体程序。

### 3.1.1 软件更新

- 应用启动后静默检查本仓库 GitHub Release 更新；无更新或暂时断网时不打断转写工作。
- 有更新时显示版本、下载进度、稍后处理和立即更新操作；安装前由用户确认。
- 只接受与内置公钥匹配的 Tauri 签名更新包，发布私钥不得进入源码仓库或普通 CI。
- `main` 分支自动执行 Windows 测试和打包；版本标签自动创建包含 NSIS、`.sig` 与 `latest.json` 的 Release。

### 3.2 云端处理

- 分离 `TranscriptionProvider` 和 `MinutesProvider`，均提供 mock。
- endpoint、model、连接/请求超时、重试次数、并发限制全部配置化。
- 支持上传、转写和总结阶段的任务取消。
- 支持安全错误分类、有限重试和 429 `Retry-After`；401/403 与结构性 4xx 不自动重试。
- 转写结果包含纯文本，并可选承载时间戳、speaker label 和 confidence；Provider 不提供时保持缺失。
- 纪要输出必须经过版本化 JSON Schema 校验，包含：
  - 会议标题
  - 会议时间
  - 参会人
  - 会议摘要
  - 主要议题
  - 关键结论
  - 决策事项
  - 待办事项
  - 风险和问题
- 参会人姓名未知时保持空数组，speaker label 不能被当作真实姓名。
- 提供标准会议、项目周会、客户沟通、课程总结、课题研究、学术讲座、人物专访、深度访谈、商业计划书、文章大纲和自适应模板；所有模板使用同一 Schema，自适应仅调整内容侧重点。

### 3.3 本地会议与展示

- SQLite 持久化会议、artifact、任务、完整转写、结构化纪要和安全错误。
- 主窗口至少提供：文件导入、批量任务、历史会议和设置入口。
- 任务列表按文件显示校验、staging、preflight、上传、转写、总结、保存和失败状态。
- 详情页展示全部纪要字段与完整转写；空字段、加载和失败均有明确状态。
- 支持按标题、日期和本地文本搜索，不为搜索调用云端。
- 支持复制摘要、结构化区块或完整转写。
- 支持导出 UTF-8 Markdown，章节顺序稳定并包含完整转写。
- 应用重启后已保存历史仍可查看；原活动任务标记 `interrupted`，允许重试或取消。

### 3.4 设置、安全与诊断

- 分别配置 ASR 与纪要 Provider 的类型、endpoint、model、超时、重试和 Key。
- 生产 Key 保存到 Windows Credential Manager；前端只能看到“已配置”，不能读回值。
- 普通日志只记录 task/artifact ID、阶段、耗时、安全错误码和 HTTP status。
- 日志、UI 错误和测试报告中禁止出现 Key、认证头、原始文件名、完整 hash、文件内容、完整转写或完整模型请求/响应。
- 用户可删除单条会议及其受管 staged copy，也可清理全部本地会议数据；用户原文件永远不在删除范围内。

## 4. 明确非目标

下列能力不阻塞 MVP：

- 应用内创建音频、实时输入、实时字幕或实时摘要。
- 默认媒体转换、压缩、切片、格式修复或编辑。
- 本地运行 ASR/LLM、GPU 管理或模型下载。
- 云端账号、团队共享、多人协作和在线管理后台。
- Word、PDF、字幕或其他富格式导出。
- 自定义 Prompt 编辑器、插件市场或自动工作流。
- macOS/Linux 支持、自动更新、代码签名和企业 MDM 自动化。
- 对所有 M4A 编码、所有 Provider 或任意大小文件的兼容承诺。

## 5. 用户流程

### 5.1 首次启动与配置

1. 无 Key 时应用仍可启动，设置和历史空状态可访问，但音频选择保持禁用并提供配置引导。
2. 用户分别配置 ASR/纪要 Provider 的 endpoint、model、超时、重试和 Key。
3. 后端保存 Key，UI 之后只显示“已配置”。
4. 用户可运行连接测试；失败只展示安全错误。

### 5.2 单文件导入

1. 用户点击导入并选择一个 WAV、MP3 或 M4A。
2. 列表展示检测到的格式、大小、可用时长和本地校验结果。
3. Rust core 完成 staging 和 hash，再按 Provider capabilities 做 preflight。
4. 用户选择纪要模板并提交任务。
5. UI 依次显示上传、转写、总结、校验和保存状态。
6. 完成后进入详情；用户查看、搜索、复制或导出 Markdown。

### 5.3 批量导入

1. 用户一次选择多个文件。
2. 每个文件独立完成校验、staging 和 preflight；无效项保留明确错误，不阻塞有效项。
3. 有效项进入持久队列，按配置的低并发处理。
4. 用户可逐项取消或重试；一个任务失败不改变其他任务结果。

### 5.4 取消、失败与恢复

1. 上传阶段取消会中止本地请求体传输。
2. 转写或总结阶段取消会中止当前请求/轮询；若远端不支持撤销，UI 必须说明只能保证本地不再等待和保存结果。
3. 可重试网络错误进入有限 `retry_wait`；401/403 提示修复凭据，429 尊重等待策略。
4. 应用重启后历史仍可查看，原活动任务显示 `interrupted`，由用户选择重试或取消。

## 6. 状态机

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
重启发现活动任务 -> interrupted -> retry / cancelled
```

规则：

- Provider 没有百分比时只显示阶段，不伪造精确进度。
- `completed` 只在转写、Schema-valid 纪要和会议记录均持久化后出现。
- 同一 artifact 的重复提交不能同时创建两个活动任务。
- 本地 `cancelled` 与远端已撤销是不同事实，必须由 Provider capability 决定展示语义。

## 7. 可验证验收标准

每一项都必须附真实命令、自动测试输出或 Windows 手工记录；未执行不得写“通过”。

### 7.1 启动与配置

- **AC-BOOT-01**：无 Key 时应用可启动，历史空状态和设置页可访问，音频入口明确禁用。
- **AC-CONFIG-01**：endpoint、model、timeout、retry 和并发均可配置，仓库无真实秘密值或内部地址。
- **AC-SECRET-01**：保存 Key 后重启仍显示“已配置”，但 UI、SQLite、普通日志和 IPC 响应不能读出值。
- **AC-PACKAGE-01**：Windows 开发启动、release build 和至少一种 NSIS/MSI 构建实际成功，记录真实产物大小。

### 7.2 文件导入

- **AC-IMPORT-01**：使用仓库现有 MP3 的相对路径完成自动导入验证；其正文和原始文件名不进入日志或快照。
- **AC-IMPORT-02**：单选和批量选择均能创建独立 artifact；批次含有效与无效文件时，有效项仍可处理。
- **AC-IMPORT-03**：WAV、MP3、M4A、MP4、MOV fixtures 覆盖合法容器、空文件、后缀伪装、损坏头、不支持编码和无音轨视频。
- **AC-IMPORT-04**：staging 与 SHA-256 使用分块 I/O；大文件测试不会按文件大小等量增长进程内存。
- **AC-IMPORT-05**：复制期间源文件变化返回 `source_file_changed`；不生成可提交 artifact。
- **AC-IMPORT-06**：删除会议只删除受管副本，不修改或删除用户原文件。
- **AC-IMPORT-07**：构建依赖、发布包和运行日志证明应用没有捆绑或调用 FFmpeg，也没有默认转换、压缩或切片。

### 7.3 Capability preflight 与 Mock 闭环

- **AC-CAP-01**：Provider capabilities 可分别接受/拒绝 WAV、MP3、M4A、MP4、MOV，并覆盖大小、时长、容器和编码限制。
- **AC-CAP-02**：明确不兼容的文件在上传前失败；未知限制不被伪装成支持或不支持。
- **AC-E2E-01**：至少一个真实音频文件通过 mock 跑通 `导入 -> 转写 -> 纪要 -> SQLite -> 详情 -> Markdown`。
- **AC-E2E-02**：mock 可确定性模拟 timeout、401、429、500、malformed response、empty transcript 和批量部分失败。

### 7.4 取消、重试与恢复

- **AC-CANCEL-01**：上传阶段取消会中止请求体传输并最终进入 `cancelled`。
- **AC-CANCEL-02**：转写阶段取消会中止请求或轮询；远端撤销能力按 capability 诚实展示。
- **AC-CANCEL-03**：总结阶段取消后不保存部分纪要，也不能进入 `completed`。
- **AC-RETRY-01**：401 不自动重试；429 遵循等待策略；timeout/500 的尝试次数不超过配置。
- **AC-TASK-01**：重复提交不会生成两个活动任务；一个批量项失败不阻塞其他项。
- **AC-RESTART-01**：重启后历史仍可查看，原活动任务变为 `interrupted` 且可重试或取消。

### 7.5 纪要、展示和导出

- **AC-SCHEMA-01**：标准纪要样例通过 JSON Schema；缺字段、错误类型和非法枚举样例被拒绝。
- **AC-EMPTY-01**：空 transcript 不调用纪要 Provider；无 speaker/timestamp 时保持诚实缺失。
- **AC-DETAIL-01**：详情页展示九类信息与完整转写，加载、空和失败状态均有前端测试。
- **AC-SEARCH-01**：标题、日期和本地文本搜索可返回保存的会议，且不发网络请求。
- **AC-EXPORT-01**：导出的 UTF-8 Markdown 章节顺序稳定，包含纪要和完整转写。

### 7.6 安全和工程验证

- **AC-LOG-01**：使用标记秘密值、文件名和 transcript 做自动扫描，普通日志/UI/测试报告均无匹配。
- **AC-DELETE-01**：受管数据删除失败时返回可见错误并登记待清理，不谎报成功。
- **AC-VALIDATE-01**：前端类型检查、Rust check、单元测试、集成测试、前端构建、Tauri build 和 installer 均真实运行并记录退出码。
- **AC-REVIEW-01**：独立 Reviewer 直接检查 diff 和代码并运行关键命令，不只引用其他 Agent 总结。

## 8. MVP 完成定义

只有以下条件同时满足才能宣布 MVP 完成：

- 所有阻塞级验收项有真实证据，失败项明确标记 BLOCKED。
- mock provider 使用至少一个真实文件完成完整闭环。
- 单文件、批量混合结果、三个云端阶段的取消和重启恢复均有验证。
- 应用启动、构建和 installer 命令实际成功。
- README 写明安装、开发、测试、构建、Provider 配置、支持格式和隐私行为。
- `docs/architecture.md`、`docs/api-contract.md`、`docs/security-review.md`、`docs/test-plan.md` 完成且范围一致。
- `.env.example` 无真实 Key，普通日志无秘密值、文件名和会议正文。
- 已知 Provider 限制、未验证接口和后续建议进入最终报告。
