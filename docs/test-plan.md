# 测试计划

> 状态：Phase 2/5 测试复审；单元测试和 Windows NSIS 构建已执行，桌面集成/E2E 仍阻塞
> 日期：2026-07-17
> 目标平台：Windows 11 x64
> 关联文档：[技术架构](./architecture.md)、[MVP 定义](./mvp.md)、[API 契约](./api-contract.md)、[安全审查](./security-review.md)

## 1. 目标与原则

本计划用于验证 MVP 主链路：

```text
单个/批量离线音视频导入 -> preflight/staging -> 内部测试转写 -> Schema-valid 纪要
-> SQLite -> 页面展示/搜索/复制 -> UTF-8 Markdown 导出
```

测试同时覆盖失败重试、取消、重复提交、重启恢复和隐私保护。规则如下：

- 自动测试优先使用离线、确定性 mock；没有真实 API Key 也能覆盖核心流程。
- 至少一份真实音频 artifact 进入 mock 端到端，但 mock 返回独立的非敏感 fixture，不识别或快照真实会议内容。
- 真实 Provider 只在 Phase 4 使用最小非敏感音频验证，响应只记录结构和安全元数据。
- 单元测试通过不能替代 Windows 真实文件对话框、文件系统行为、打包和安装手测。
- 每条验收结果必须有命令退出码、测试数或手工记录；未执行写 `NOT RUN`，受环境阻塞写 `BLOCKED`。
- 不在日志、快照、报告、截图或 Agent 消息中包含 API Key、Authorization、会议原文、Prompt、音频 bytes 或本地绝对敏感路径。
- 当前曾报告过明文测试密钥事件；完成供应商侧吊销/轮换前，真实 Provider 测试全部 `BLOCKED`。

## 2. 当前 Phase 2/5 实际结果

### 2.1 已执行

- 直接审查前端、Rust、配置、Tauri capability、Provider、ingest、SQLite 与 mock 实现。
- `pnpm typecheck`：退出码 0。
- `pnpm test`：退出码 0；3 个 test files、19 个 tests 全部通过，包括配置引导、音视频批量入口、自动更新、11 模板和 StrictMode Markdown 预览回归。
- `cargo test --manifest-path .\src-tauri\Cargo.toml --all-targets --all-features`：退出码 0；最终 96 个 Rust tests 全部通过。仓库本地真实 MP3、MP4/MOV 结构夹具和无音轨拒绝用例实际执行通过。
- `MEETING_DESK_TEST_VIDEO=<仓库外生成文件>` 定向测试：退出码 0；FFmpeg 生成的 1 秒 H.264 + AAC MP4 经 `ffprobe` 确认为视频轨 + 音频轨，并通过真实 importer；文件未进入仓库。
- `cargo clippy --manifest-path .\src-tauri\Cargo.toml --all-targets --all-features -- -D warnings`：退出码 0。
- `pnpm build`：退出码 0；Vite 生产构建成功，最终主 JS 约 245 kB、gzip 约 76 kB。
- `pnpm tauri:build`：退出码 0，使用无私钥 CI 配置生成 0.2.0 release exe 和 NSIS 安装器；应用实际启动 5 秒保持运行。
- `pnpm tauri:build:release`：退出码 0，使用仓库外无密码 updater 私钥生成 4,273,464-byte NSIS 安装器和 424-byte `.sig`；GitHub Actions Secret 已配置。安装器 Authenticode 状态仍为 `NotSigned`。
- `cargo fmt --check --manifest-path .\src-tauri\Cargo.toml`：退出码 0。
- `pnpm test:integration`：退出码 0；1 个直接 Rust 集成用例通过，覆盖合成 WAV -> ingest -> MockProvider -> 纪要校验 -> 内存 SQLite -> staging 清理。
- `pnpm audit --prod --registry=https://registry.npmjs.org`：退出码 0，未报告已知生产依赖漏洞。默认 npm 镜像缺少 audit endpoint；`cargo-audit` 未安装。
- 高置信度 secret scan：扫描 144 个工作区文本文件，仅命中 2 个明确无效的测试 sentinel 文件，真实供应商凭据命中 0；`.env.example` Key 非空检查为 false；Git 跟踪音频数 0。
- release 产物/bundle 中未发现 `.env*`、音频、SQLite/DB 或日志文件；应用和安装器均为 `NotSigned`。

### 2.2 未执行与阻塞

| 项目 | 当前状态 | 真实结果/下一步 |
| --- | --- | --- |
| TypeScript 类型检查 | `PASS` | `tsc --noEmit` 通过 |
| 前端组件/测试客户端 | `PASS` | 19/19 通过；含 updater 和视频批量入口，只覆盖浏览器测试客户端，不代表 Tauri desktop client |
| Rust 单元测试 | `PASS` | 96/96 通过，含 MP4/MOV、无音轨视频、Provider 就绪状态、11 模板注册表、自适应 Prompt，并继续覆盖重试上限与取消终态 |
| Rust clippy | `PASS` | `-D warnings` 通过 |
| Rust format | `PASS` | `cargo fmt --check` 通过 |
| 前端生产构建 | `PASS` | Vite build 成功 |
| Tauri release/NSIS | `PASS` | 无私钥 CI 构建与签名 Release 构建均成功，release exe 启动通过；尚未安装或卸载实测 |
| 独立集成测试 | `PARTIAL` | 脚本存在且 1/1 通过；直接调用 Rust、内存 SQLite，不是 Tauri IPC/UI 或磁盘重启 E2E |
| 桌面 mock E2E | `PARTIAL` | 前端 15 个命令已全部注册；浏览器已实测配置引导、11 模板和 Markdown 预览，尚未在真实 WebView/IPC 跑完整闭环 |
| Credential Manager 实测 | `NOT RUN` | 空 Key 删除缺陷已在代码层修复；尚无 Windows Credential Manager/IPC/重启回归 |
| Windows 文件对话框/拖放 | `PARTIAL` | 裸路径拖放命令已移除，桌面只支持系统选择器；原生对话框、只读/锁定文件未手测 |
| SQLite 重启恢复 | `PARTIAL` | 活动态/CancelRequested 恢复、重试上限有内存测试；系统对话框重选已接回原任务，但磁盘库/WAL、历史重启和真实 IPC 重选未验 |
| 真实 Provider | `BLOCKED` | 字段/adapter 未验证；历史测试密钥轮换未确认 |
| Git diff/历史审查 | `PASS` | 已存在远端基线提交；本轮执行 `git diff --check`、变更统计、跟踪文件与高置信度 secret scan |
| Windows 安装权限 | `NOT RUN` | 安装器未签名，未实测 UAC、安装模式、ACL、SmartScreen 和卸载残留 |

### 2.3 已证实覆盖与覆盖缺口

| 领域 | 已证实 | 尚未证实/缺口 |
| --- | --- | --- |
| Ingest | synthetic WAV/MP3/M4A、带音轨 MP4/MOV、无音轨 MP4、真实本地 MP3、零字节、损坏、扩展/容器不符、大小限制、批量部分失败、流式 hash 去重、源内容不变、`.part` 失败清理、显式 release、成功终态清理、启动清理 | 真实长视频、原生对话框、只读/锁定/消失/替换竞态、复制中取消、cleanup_pending、多残留部分失败、卸载残留 |
| Provider | mock success/空 transcript/可取消 delay；401 终止、429 重试、5xx 有界重试、pre-send 网络重试、unknown outcome 重放拒绝、malformed response、Debug secret redaction | 403/413、response too large、真实 reqwest timeout/TLS、真实 Provider、远端取消、完整 task orchestrator |
| Secret | 前端 mock 测试验证输入清空/不回显；Credential wrapper Debug redaction；空 Key 后端忽略；`.env.example` 空值 | Windows Credential Manager 写/读/替换/删除/重启；IPC/内存生命周期；旧 Key 轮换 |
| SQLite | 参数化 CRUD、转写/纪要事务、级联删除的内存测试 | 磁盘库、WAL/SHM、ACL、损坏/磁盘满、重启恢复、受管 artifact 联动删除 |
| UI | 导入空态、未配置时禁用媒体、音频/视频批量列表、部分错误、任务取消确认、重新选择、详情/复制、无采集 UI | Tauri command 集成、真实错误码、导出、持久化重启、键盘/缩放/高对比度 |
| 配置引导 | 单侧缺失、两侧真实就绪、打开设置、DeepSeek/百炼/第三方兼容切换 | Windows Credential Manager 重启后就绪状态与真实 Provider 契约验证 |
| Markdown 预览 | 后端预览与导出共用渲染器、标题/列表/表格/逐字稿、禁用 raw HTML | 超长逐字稿渲染性能、复制选择和打印布局 |
| Packaging | release exe 与 NSIS 成功；bundle 未发现明显敏感附带文件 | 实际安装/启动/卸载、签名、UAC/权限、WebView2 缺失、SmartScreen/EDR |
| Updater | 前端静默检查/发现版本/下载进度/重启组件测试；Tauri 公钥、固定 GitHub endpoint、最小 capability；CI 与标签发布工作流；Release 三项资产已生成 | **BLOCKED：仓库当前为 Private，未登录客户端访问 latest.json 返回 404**；改为 Public 或迁移公开分发端点后，再做旧版到新版、代理/断网/损坏签名回归 |

### 2.4 Lead 必须关闭的测试门槛

1. **P0/Critical**：提供不含值的旧测试密钥吊销/轮换确认；关闭前禁止真实 Provider 测试。
2. **P1/High（代码已修）**：对空 `apiKey` 不替换行为增加“首次保存、留空不替换、替换、显式删除、环境变量回退、重启”Credential Manager/IPC 测试。
3. **P1/High（已关闭）**：裸 `PathBuf` 拖放命令已移除；桌面只允许系统选择器。恢复拖放前必须重新做授权设计。
4. **P1/High（部分关闭）**：release/终态/启动清理已实现；仍需覆盖锁定文件、持久化 `cleanup_pending`、多残留部分失败、程序崩溃和卸载。
5. **P1/High（代码已修）**：15 个前端命令已注册；下一门槛是使用真实 Tauri client 跑音频 + mock Provider + 磁盘 SQLite + 重选/详情/搜索/预览/导出 E2E。
6. **P1/High（代码已修）**：endpoint 已拒绝 userinfo/query/fragment；补参数化测试并确认拒绝路径不写 SQLite。
7. **P1/High（代码已修）**：取消保存回滚、取消/重试状态门、活动 token 防覆盖、尝试次数上限、重启状态恢复和系统对话框重绑旧任务已实现；补真实同步屏障取消竞态和并发 barrier 单飞测试。
8. **P2/Medium**：扩展现有 `test:integration`，验证磁盘 SQLite、WAL、取消 late success、批量部分失败、重启、原子 Markdown 导出和敏感信息扫描。
9. **P2/Medium**：完成 Windows 普通用户安装/启动/卸载、ACL、UAC/安装模式、WebView2、SmartScreen/签名及 Rust 依赖审计。

## 3. 测试分层与责任

| 层级 | 主要对象 | 工具/方法 | 责任 |
| --- | --- | --- | --- |
| 前端单元/组件 | 路由、状态、表单、单个/批量导入、任务/详情、错误/空状态 | Vitest + Testing Library，必要时 axe | Agent 2 |
| Rust 单元 | 文件 ingest、Provider 错误/重试、Schema parser、repository、导出 | `cargo test` | Agent 3/4/5 + Lead |
| Contract | mock/codec、JSON Schema、IPC DTO、Markdown 格式 | Rust integration + shared fixtures | Agent 4/5 + Lead |
| 集成 | task orchestrator、SQLite、mock provider、取消/重启/删除 | 临时 app-data + mock clock/server | Lead |
| 桌面 E2E | Tauri UI 到 Rust command 的完整闭环 | Tauri/WebDriver 或受控桌面测试；不足处手测 | Lead + Agent 6 |
| Windows 实机 | 系统文件对话框、WAV/MP3/M4A/MP4/MOV 导入、只读源文件、锁定/移动文件、安装包、自动更新 | Windows 11 x64 手工验收 + 工具校验 | Lead + Agent 6 |
| 安全 | secret/正文扫描、capabilities、ACL、路径、日志、供应链 | 静态扫描 + sentinel + PowerShell ACL/文件检查 | Agent 6 + Reviewer |
| 独立审查 | diff、核心流程、安全、并发、构建、测试质量 | Reviewer 直接看代码并运行关键命令 | Phase 6 Reviewer |

## 4. 测试环境矩阵

### 4.1 最低环境

| 环境 | 用途 | 必需状态 |
| --- | --- | --- |
| Windows 11 x64，普通用户 | 主开发、功能、安装/卸载 | 必测 |
| Rust stable `x86_64-pc-windows-msvc` | Rust/Tauri 构建 | 必测 |
| 项目锁定的 pnpm 与 Node 版本 | 前端构建/测试 | 必测；版本写入 `packageManager`/README |
| WebView2 Evergreen 当前企业支持版本 | UI 运行 | 必测 |
| WAV、MP3、M4A、MP4、MOV 各一份非敏感样例 | 格式与批量导入 | 必测；视频同时准备“有音轨”和“无音轨”样例；真实 Provider 未验证前只证明本地 preflight/内部测试流程 |
| 只读、锁定、移动/删除中的测试文件 | 外部源保护与竞态 | 必测；只在隔离测试目录操作 |
| 无网络/受控 mock server | 失败、重试、取消 | 必测 |

### 4.2 建议兼容环境

- Windows 显示缩放 100% 与 200%，高对比度、减少动画。
- Unicode Windows 用户名和含空格的用户目录。
- 非管理员用户、企业代理/自签 CA（如实际目标环境需要）。
- WebView2 缺失/损坏或网络不可下载运行时的安装行为。
- 本地磁盘、可移动磁盘、UNC/OneDrive/企业同步目录作为扩展矩阵；未测不得承诺兼容。

## 5. 测试数据与隐私

### 5.1 数据集

| 类型 | 用途 | 要求 |
| --- | --- | --- |
| 生成的短 PCM WAV（静音/正弦/公开朗读） | 容器、时长、零字节/内容预检 | 无企业内容，可提交测试 fixture |
| 损坏/截断 WAV、伪扩展名文件 | 解码和 preflight | 小尺寸、确定性 |
| 超大/超长逻辑 fixture | 大小上限 | 优先 sparse/mock stream，不提交巨大二进制 |
| 固定 mock transcript/minutes | Provider/Schema/UI | 人工非敏感文本，不来自真实 MP3 |
| 仓库根目录本地 MP3 | 真实音频 artifact 的导入与 mock E2E | 以相对路径或显式环境变量引用；不复制原文、哈希、路径到日志/快照 |
| sentinel secret/transcript | 泄露扫描 | 明显无效、不会被真实 Provider 接受；不模仿真实 key 格式 |

### 5.2 禁止事项

- 测试代码不得自动删除、移动或修改仓库根目录的用户 MP3。
- 不把真实 MP3 复制到 fixture、报告、构建产物或临时目录之外；若应用导入策略需要受管副本，测试结束按策略清理。
- snapshot 不包含完整 transcript/minutes；只验证结构、计数、固定短非敏感片段或摘要 hash（hash 本身也不记录真实资产）。
- 测试失败消息不得打印 request/response body、完整 DTO、绝对路径或环境变量值。

## 6. 阶段 Gate

### Phase 1：骨架

- 应用在无 Key、无网络下启动，四个基础路由可访问。
- mock provider success 与主要失败场景可编译、可测试、可取消。
- 配置校验、统一错误、日志白名单、最小 Tauri capability/CSP 有基线。
- 根目录提供 PowerShell 兼容的统一脚本；类型检查、单元测试和开发/release build 实际运行。
- 离线文件导入 PoC 必须证明 WAV/MP3/M4A/MP4/MOV 的系统文件选择、preflight、只读源保护、staging 与取消清理；失败项给出 Go/BLOCKED 结果。

### Phase 2：模块

- UI、音频、Provider、纪要 Schema、SQLite 各自单元/模块测试通过。
- 每个 Agent 报告命令、退出码、测试数、已知问题；不能仅报告“已实现”。

### Phase 3：集成

- 使用真实音频 artifact + mock provider 跑通完整闭环。
- 取消、重复提交、重启恢复、保存事务、导出与删除通过集成测试。

### Phase 4：真实 API

- `SEC-P0-001` 关闭后才能开始。
- 只验证已明确 endpoint/字段的 adapter；未知差异保持 `unverified`。
- 真实调用结果只记脱敏结构证据；不可访问则 `BLOCKED`，继续保留 mock。

### Phase 5/6：发布质量与独立审查

- 所有规定命令、Windows 手测、release build、至少一种 NSIS/MSI 包实际完成。
- Reviewer 直接检查 Git diff/核心代码并复跑关键命令。

## 7. 功能与可靠性测试矩阵

状态栏在执行后填写 `PASS`、`FAIL`、`BLOCKED` 或 `NOT RUN`，不得预填通过。

### 7.1 启动、路由与配置

| ID | 场景 | 层级 | 关键步骤 | 预期 |
| --- | --- | --- | --- | --- |
| `BOOT-001` | 无 Key 首次启动 | E2E/手工 | 清空测试 app-data，启动应用 | 启动成功；历史为空；设置可用；音频选择禁用；无错误泄露 |
| `BOOT-002` | app-data 不可写 | 集成/手工 | 使用受控只读目录或注入存储失败 | 安全错误；不崩溃；不谎报已保存 |
| `UI-001` | 基础路由 | 组件/E2E | 依次进入会议、导入、任务、设置 | 页面稳定；焦点移到标题；无网络请求 |
| `UI-002` | 加载/空/错误状态 | 组件 | 为每页注入三类状态 | 对应内容清楚；导航仍可使用 |
| `CFG-001` | endpoint/model/timeout/retry 配置 | 单元/组件 | 输入合法与边界值 | 合法保存；0/负数/溢出/空 model 被拒绝 |
| `CFG-002` | 生产 HTTP endpoint | 单元 | 配置非 localhost HTTP | 拒绝；开发显式 localhost mock 可用 |
| `CFG-003` | URL userinfo/query/fragment | 单元 | 构造各种 URL | userinfo/fragment 拒绝；query 按 adapter policy 拒绝 |
| `CFG-004` | Key 保存后重启 | 集成/手工 | 通过 Windows Credential Manager 保存 sentinel，重启 | 只显示已配置；UI/SQLite/IPC 不含原值 |
| `CFG-005` | Key 写入失败 | 集成 | SecretStore 注入失败 | 不回退明文；状态仍未配置；错误安全 |
| `CFG-006` | 替换/删除 Key | 集成 | 覆盖 sentinel，再删除 | 旧值不可读取；删除后未配置；日志无值 |
| `CFG-007` | 测试连接失败 | 组件/集成 | mock 401/timeout | 只显示安全码/建议；不显示 header/body |

### 7.2 离线文件选择、preflight 与 staging

| ID | 场景 | 层级 | 关键步骤 | 预期 |
| --- | --- | --- | --- | --- |
| `IMP-001` | 单文件导入 | 集成/E2E | 通过系统对话框选择受支持的 WAV、MP3、M4A、MP4、MOV | preflight 后创建一个任务；前端只获得安全元数据 |
| `IMP-002` | 批量导入部分失败 | 集成/E2E | 混合合法、空、损坏、超大和不支持格式文件 | 合法文件继续；失败逐项显示；批次不假装全成/全败 |
| `IMP-003` | 多次追加累计超限 | Rust 单元/集成 | 分多次选择后累计超过批次文件数或总大小限制 | 提交边界按完整逻辑批次拒绝，不启动任何子任务 |
| `IMP-004` | 活动任务重复导入 | Rust 单元/集成 | 正在处理的 artifact 再次被选择并移除候选 | 重复项不可提交且不暴露可释放 artifact ID；活动任务文件保持可用 |
| `IMP-003` | 0 字节音频 | 单元 | 导入空文件 | preflight `empty_audio`，Provider 调用 0 次 |
| `IMP-004` | 损坏/截断音频 | 单元/集成 | 导入 fixture | `corrupt_audio`；不崩溃、不上传 |
| `IMP-005` | 扩展名/MIME 不一致 | 单元 | MP3 后缀放非音频数据 | 不只信扩展名；安全拒绝 |
| `IMP-006` | 超大/超长音频 | 单元/集成 | 超过已验证 capability limit | `unsupported_audio`/本地大小错误；无网络请求 |
| `IMP-007` | 上限未知 | 单元 | capability 缺失最大值 | UI 不显示虚构上限；由 adapter/Provider 实际处理 |
| `IMP-008` | 重复导入/重复点击 | 集成 | 同一 artifact/config 并发提交 | 返回同一活动 task；只发一个 operation |
| `IMP-009` | 用户源文件只读保护 | 集成/手工 | 对只读源执行处理、取消、删除会议和全部清理 | 流程不要求写源文件；外部源不被修改、移动或删除 |
| `IMP-010` | 本地真实 MP3 + mock | E2E | 以相对路径/显式配置导入仓库测试资产 | artifact 验证和完整闭环成功；日志/报告不含原文 |
| `IMP-011` | WAV/MP3/M4A/MP4/MOV 格式矩阵 | Contract/集成 | 对每种格式执行容器、音轨、MIME、时长与 Provider capability preflight | 支持能力来自 adapter；不支持格式和无音轨视频在上传前拒绝 |
| `IMP-012` | 文件选择取消 | 组件/E2E | 关闭系统文件对话框 | 不创建 artifact/task；不显示失败 Toast |
| `IMP-013` | 无法读取/被锁定文件 | 集成/手工 | 选择无读取权限或被独占锁定的测试文件 | 安全错误；无 staging/task；绝对路径不进日志 |
| `IMP-014` | preflight 后源文件被替换 | 并发集成 | 预检后、上传前替换内容或改变大小/时间 | 身份复核失败；不上传已变化文件；提示重新选择 |
| `IMP-015` | preflight 后源文件消失 | 并发集成 | 预检后移动/删除隔离测试文件 | `source_unavailable`；任务不完成；不影响批次其他文件 |
| `IMP-016` | 流式 hash | 单元/集成 | 对大 fixture 计算 SHA-256 并监测内存 | hash 正确且内存有界；hash/路径不进普通日志 |
| `IMP-017` | hash/去重边界 | 集成 | 同内容不同文件名、同文件不同配置 | 活动任务按定义去重；不同配置不误合并；不凭文件名判断 |
| `IMP-018` | staging 正常完成 | 集成 | 复制到隔离 app-data 后校验并发布 artifact | 使用 opaque 名；完整后才可上传；源文件保持不变 |
| `IMP-019` | staging 磁盘满/写失败 | 集成 | 注入中途写失败 | 不发布部分 artifact；登记/清理残留；批次其他文件继续 |
| `IMP-020` | staging 期间取消 | 集成 | 复制中触发取消 | 有界时间内停止；终态 cancelled；残留删除或 cleanup pending |
| `IMP-021` | 程序退出留下 staging | 集成/手工 | 复制中终止测试进程后重启 | 只识别本应用残留；不当成完整文件；按策略清理 |
| `IMP-022` | 启动清理范围 | 集成 | 受管残留旁放置无关文件并启动清理 | 仅清理 manifest/数据库证明归属的 staging，不使用宽泛 glob |
| `IMP-023` | 前端数据边界 | 静态/集成 | 检查文件选择/任务 IPC 与事件 | 不含音频 bytes、hash 或可复用绝对路径；只含 opaque ID/安全元数据 |

### 7.3 Provider、网络、重试和限流

| ID | 场景 | 层级 | 关键步骤 | 预期 |
| --- | --- | --- | --- | --- |
| `API-001` | mock success | Contract | ASR 与 minutes 依次成功 | 返回稳定 DTO；Schema valid；调用记录无正文 |
| `API-002` | 无 Provider 配置 | 单元 | 创建真实类型任务但无 credentialRef | `provider_not_configured`；请求 0 次 |
| `NET-001` | DNS/连接拒绝/断网 | 集成 | mock transport 注入各错误 | 映射 `network_unavailable`；只在 replay-safe 时有限重试 |
| `NET-002` | TLS 证书错误 | 集成 | 本地受控无效证书 | 安全失败；没有忽略验证的回退 |
| `NET-003` | connect timeout | 虚拟时钟/集成 | body 未发送时超时 | `connect_timeout`，outcome NotSent；次数不超配置 |
| `NET-004` | 上传后 timeout | 集成 | body 已发送，响应延迟 | outcome Unknown；无已验证幂等时不自动重放 |
| `NET-005` | overall timeout | 虚拟时钟 | 让重试/等待超过总 deadline | `operation_timeout`；不再重试；资源释放 |
| `API-003` | HTTP 401 | Contract/集成 | mock 返回 401 | 自动重试 0 次；UI 前往设置；不显示 body |
| `API-004` | HTTP 403 | Contract/集成 | mock 返回 403 | 自动重试 0 次；安全错误 |
| `API-005` | HTTP 429 后成功 | 虚拟时钟/集成 | 前 N 次 429 + delta Retry-After | 有上限等待，attempt 正确，随后成功 |
| `API-006` | 429 HTTP-date/非法值 | 单元 | 两类 header | 正确解析或回退本地退避；原值不进错误 |
| `API-007` | 429 shared cooldown | 并发集成 | 同 credential 多任务触发 429 | 同 key 共享冷却；等待可取消；无请求风暴 |
| `API-008` | HTTP 500/502/503/504 | Contract | replay-safe 与 unsafe 各测 | safe 有限重试；unsafe 不重放；attempt 上限正确 |
| `API-009` | 结构性 4xx/413 | Contract | 返回 400/404/405/409/413/422 | 默认不重试；稳定错误分类 |
| `API-010` | malformed response | Contract | 非法 JSON/缺字段/错误类型 | `invalid_provider_response`；raw body 不经 IPC |
| `API-011` | oversized response | 集成 | 响应超过上限 | 停止读取；`response_too_large`；内存有界 |
| `API-012` | 空 transcript | Contract/集成 | ASR 返回空白 | `empty_transcript`；minutes 调用次数 0 |
| `API-013` | 取消排队/上传/退避/轮询/生成 | 集成 | 在各阶段触发 token | 及时 cancelled；late success 丢弃；无完成写入 |
| `API-014` | 远端不可取消 | 集成/UI | 异步 mock remote cancel 失败 | 本地 cancelled；状态 `remote_state_unknown`；提示可能计费 |
| `API-015` | 并发限制与公平性 | 压力/虚拟时钟 | 批量任务超过 semaphore | 不超上限；FIFO/等价公平；等待可取消 |
| `API-016` | endpoint redirect | 集成 | 同源/跨源/HTTPS 到 HTTP 重定向 | 按 policy 拒绝或安全跟随；跨源不转发凭据 |
| `API-017` | 最大重试边界 | 单元 | maxRetries 为 0、1、上限 | 总 attempt=`1+maxRetries`；配置上限生效 |
| `API-018` | 真实字段未知 | Contract | adapter 标记 Unverified | 不将未知能力报告为支持，不生成虚构字段 |

### 7.4 会议纪要、Prompt 与展示

| ID | 场景 | 层级 | 关键步骤 | 预期 |
| --- | --- | --- | --- | --- |
| `MIN-001` | 标准样例 | Schema/解析 | 校验标准 fixture | 通过唯一版本化 JSON Schema |
| `MIN-002` | 缺字段/错类型/额外字段 | Schema | 各类 invalid fixture | 被拒绝，错误不含完整模型输出 |
| `MIN-003` | 空 transcript | 单元 | 传空白 | 不调用模型，稳定错误 |
| `MIN-004` | 无 speaker/timestamp/confidence | 单元/UI | 可选字段全部缺失 | 不伪造；UI 诚实降级 |
| `MIN-005` | 低置信度 | 单元 | 含合法 confidence | Prompt/结果保留不确定性；不臆造身份 |
| `MIN-006` | Prompt 注入文本 | 单元/Contract | transcript 含伪指令/JSON/HTML | 作为数据处理；输出仍过 Schema；不调用工具/外部 URL |
| `MIN-007` | 超长 transcript | 单元/集成 | 超过 provider input capability | 明确拒绝或经定义的分段策略；不截断冒充完整 |
| `MIN-008` | 非法模型 JSON | Contract | malformed/Schema-invalid mock | 任务不完成；有限 repair/retry 符合策略 |
| `MIN-009` | 恶意 Markdown/HTML | 组件/E2E | fixture 含 script/event/link | 应用只显示文本；无脚本执行或自动导航 |
| `UI-DETAIL-001` | 九类信息与全文 | 组件/E2E | 加载完整、部分空和失败数据 | 所有区块稳定；空值诚实；页签失败隔离 |
| `UI-COPY-001` | 复制摘要/区块/全文 | 组件/手工 | 触发复制与失败 | 内容正确；反馈不重复正文；日志/snapshot 无全文 |
| `SEARCH-001` | 标题/日期/本地正文搜索 | 集成/E2E | 保存多条后搜索 | 结果正确；不调用 Provider；过期请求不覆盖新结果 |

### 7.5 任务、SQLite 与重启恢复

| ID | 场景 | 层级 | 关键步骤 | 预期 |
| --- | --- | --- | --- | --- |
| `TASK-001` | 正常状态流 | 单元/集成 | 走 queued 到 completed | 只允许合法转换；真实阶段持久化 |
| `TASK-002` | completed 保存顺序 | 集成 | 在 transcript/minutes/save 各点注入失败 | 仅全部事务完成后 completed |
| `TASK-003` | 取消与 late success | 并发集成 | cancel 后返回成功 | 终态 cancelled；结果不写库 |
| `TASK-004` | 重复提交单飞 | 并发集成 | 多线程相同 dedupe key | 一个活动 task/operation；无重复账单 |
| `TASK-005` | 用户主动重新生成 | 集成 | 已完成后点击重新生成 | 新 operation/task；不静默覆盖旧记录 |
| `DB-001` | migration 新装/升级 | 集成 | 空库和上一 schema 版本升级 | 结果一致；失败不破坏原库 |
| `DB-002` | 应用重启恢复 | 集成/E2E | 完成记录后重启 | 历史、详情、搜索仍可读 |
| `DB-003` | 活动任务重启 | 集成/E2E | 中断上传/总结后重启 | 标记 interrupted；可重试/取消；不假装完成 |
| `DB-004` | SQLite 锁/磁盘满 | 集成 | 注入 busy/full | 有界等待/安全失败；状态不矛盾 |
| `DB-005` | 数据库损坏 | 集成/手工 | 使用损坏副本 | 明确恢复错误；不静默创建空库掩盖历史 |
| `DB-006` | WAL/SHM 隐私 | 安全/集成 | 写入 sentinel 后退出/清理 | 按保留策略处理；扫描范围包括 sidecar |
| `DB-007` | 搜索注入/资源上限 | 单元/集成 | 特殊字符、超长查询 | 参数化；无 SQL 注入；结果/时间有界 |

### 7.6 导出、删除与保留

| ID | 场景 | 层级 | 关键步骤 | 预期 |
| --- | --- | --- | --- | --- |
| `EXP-001` | 标准 Markdown | 单元/E2E | 导出完整会议 | UTF-8；章节顺序稳定；包含纪要与全文 |
| `EXP-002` | 中文/emoji/特殊字符 | 单元 | fixture 导出后重新读取 | 无乱码；结构不被意外截断 |
| `EXP-003` | 用户取消保存 | E2E/手工 | 关闭保存对话框 | 不报错、不留临时文件 |
| `EXP-004` | 写入失败/已有目标 | 集成 | 注入权限/磁盘失败 | 已有文件不被半覆盖；错误安全；临时文件清理 |
| `EXP-005` | 路径/重解析点/UNC | 安全/手工 | 受控路径矩阵 | 无路径穿越；只写用户选择目标；同步/远程位置有隐私提示 |
| `DEL-001` | 删除单条会议 | 集成/E2E | 删除含受管音频的会议 | DB、索引、受管 artifact 清理；外部源/导出不删 |
| `DEL-002` | 文件删除失败 | 集成 | 锁住 artifact 后删除 | cleanup pending；UI 不谎报完全清理；可重试 |
| `DEL-003` | 全部本地数据 | E2E/手工 | 二次确认后清理 | 会议数据清理；凭据是否删除单独选择 |
| `DEL-004` | 取消任务清理 | 集成 | 上传/总结时取消 | staging 按策略清理；远端未知状态说明清楚 |
| `RET-001` | 启动清理 | 集成 | 构造过期 `.part` 和无关文件 | 只清理有所有权证据的 artifact，不碰无关文件 |
| `RET-002` | 保留期边界 | 虚拟时钟 | 到期前后运行清理 | 仅到期受管数据删除；活跃任务不删 |

## 8. 安全专项矩阵

| ID | 场景 | 方法 | 通过标准 |
| --- | --- | --- | --- |
| `SEC-001` | 当前仓库 secret scan | 只输出文件名的高置信度扫描 + 人工配置审查 | 无真实 Key/Token/Cookie/内部地址；例外均为明确无效 sentinel |
| `SEC-002` | Git 历史扫描 | 扫描 refs、objects、diff | 基线提交存在；本轮提交前扫描当前 diff 与将要跟踪的文件，未把 updater 私钥、API Key 或媒体文件纳入提交 |
| `SEC-003` | 已暴露 Key 轮换 | 供应商侧不含值的确认 | 旧值失效后才可关闭，不通过“文件已删”关闭 |
| `LOG-001` | secret sentinel | 跑全部 mock 错误并扫描 stdout/stderr/log/report/snapshot | sentinel 原值与 Authorization 不出现 |
| `LOG-002` | transcript sentinel | 跑转写、Schema 失败、panic/error paths | 普通日志、错误、IPC event、报告无正文 |
| `LOG-003` | 文件名/路径 sentinel | 使用敏感文件名/Unicode 路径 | 普通日志只出现 artifact ID，不出现名/绝对路径 |
| `IPC-001` | public settings | 捕获 command response/event | 只有 `secretConfigured`/reference；无 secret value |
| `IPC-002` | error sanitization | 底层错误含 header/body/path sentinel | WebView 只见 safeMessage/code/status |
| `TAURI-001` | capability 最小权限 | 审查 capability JSON 与生成清单 | 无通用 shell、任意 fs/http、远程 origin API access |
| `TAURI-002` | CSP/远程导航 | 静态审查 + 注入链接/HTML | 远程脚本/内联执行被阻止；会议文本不执行 |
| `TAURI-003` | release DevTools/debug command | release 包实测/静态审查 | DevTools 关闭；测试/secret/path 调试命令不存在 |
| `FILE-001` | artifact ID 路径穿越 | 构造 `..`、绝对路径、设备路径、重解析点 | 拒绝；不能读写受管根之外 |
| `FILE-002` | app-data ACL | PowerShell/Windows ACL 检查 | 普通其他用户无读取；当前用户/系统符合设计 |
| `FILE-003` | 临时文件生命周期 | success/cancel/failure/crash/uninstall 矩阵 | 状态与实际残留一致；失败有 cleanup pending |
| `NET-SEC-001` | redirect credential leak | 受控双 host server | Authorization 不跨源；HTTP downgrade 拒绝 |
| `NET-SEC-002` | TLS 验证 | 自签/过期/主机名错误证书 | 安全失败，无“忽略”回退 |
| `NET-SEC-003` | response/body/resource limit | 超大 header/body、慢速流 | 内存/时间有界；连接取消；日志无 body |
| `DB-SEC-001` | Key 不入库 | 保存配置后查询测试 DB 的 schema/values | 无 secret 原值；只有 reference/布尔状态 |
| `DB-SEC-002` | 正文仅在授权本地位置 | 扫描 app-data/browser storage/temp | 不进入 localStorage/sessionStorage/cache/无关目录 |
| `DEP-001` | 依赖审计 | pnpm/Cargo 锁文件与审计工具 | 高危项有修复或书面接受；不夸大审计覆盖 |
| `PKG-SEC-001` | 安装包内容 | 解包/安装后检查 | 不含 `.env`、测试 Key、真实音频、测试 DB、调试日志 |

## 9. Windows 构建、打包与手工验收

### 9.1 自动/命令验证

从仓库根目录在 PowerShell 运行并记录版本、命令、退出码和测试数。当前 `package.json` 已提供除 `test:integration` 外的以下入口：

```powershell
pnpm install --frozen-lockfile
pnpm typecheck
pnpm test
pnpm test:integration
pnpm build
pnpm tauri:build
```

Rust 模块还应直接验证：

```powershell
cargo fmt --check --manifest-path .\src-tauri\Cargo.toml
cargo clippy --manifest-path .\src-tauri\Cargo.toml --all-targets --all-features -- -D warnings
cargo test --manifest-path .\src-tauri\Cargo.toml --all-targets
```

本轮已实际运行上述 typecheck、test、test:integration、build、Tauri build、fmt、clippy 和 Rust test；结果见第 2 节。当前 integration 只是一个直接 Rust 闭环，不能标为桌面 E2E。`pnpm install --frozen-lockfile` 本轮未重新运行，不得标记通过。

### 9.2 打包矩阵

| ID | 场景 | 预期 |
| --- | --- | --- |
| `PKG-001` | release build | 退出码 0；记录 exe/installer 大小；无调试资源 |
| `PKG-002` | NSIS 或 MSI 至少一种 | 实际生成并安装；不要求管理员则以普通用户完成 |
| `PKG-003` | 安装、首次启动、升级、卸载 | 路径正确；历史/凭据保留语义明确；卸载残留按文档 |
| `PKG-004` | WebView2 已安装 | 应用正常启动 |
| `PKG-005` | WebView2 缺失/离线 | 安装器按选择的 bootstrapper/offline 策略给明确错误或完成安装 |
| `PKG-006` | Unicode/空格用户路径 | 安装、文件导入、DB、导出正常 |
| `PKG-007` | Windows Defender/SmartScreen | 记录事实结果；未签名警告列为已知限制，不伪称可信发布 |
| `PKG-008` | 依赖开发机 ffmpeg | 在无 ffmpeg PATH 的干净环境运行 | WAV/MP3/M4A/MP4/MOV 导入按声明能力工作；当前实现不得调用外部 ffmpeg |

### 9.3 Windows 手工验收清单

每次记录测试日期、应用 build/commit、Windows build、样例格式与来源类型（不记录敏感文件名/路径）、操作人、结果、证据位置和问题 ID。

- [x] 无 API Key 启动，进入导入和设置；音频选择保持禁用并显示双服务配置引导（浏览器实测）。
- [ ] 通过系统文件对话框分别导入非敏感 WAV、MP3、M4A、MP4、MOV，preflight 结果与声明能力一致。
- [ ] 批量选择合法、零字节、损坏、超大和不支持格式文件，单个失败不阻塞其他任务。
- [ ] 只读外部源可处理；处理、取消、删除会议和清理数据都不修改、移动或删除源文件。
- [ ] 文件无读取权限、被独占锁定、preflight 后被替换或消失时显示安全错误，不崩溃、不上传错误内容。
- [ ] staging copy 正常、磁盘写失败、复制中取消和程序退出后的残留状态均与实际文件一致。
- [ ] hash/preflight 使用有界流式 I/O；路径、hash、音频内容不进入 UI、日志或测试报告。
- [ ] 内部测试链路完整跑通保存、详情、搜索、复制和 Markdown 导出（仍需真实 Tauri WebView/IPC 实测）。
- [ ] 断网、connect/request/overall timeout、401、429、500 页面行为与重试次数正确。
- [ ] staging/上传/等待/转写/总结阶段取消，最终 cancelled；late success 不出现。
- [ ] 快速重复提交只产生一个活动任务。
- [ ] 处理途中关闭/终止应用，重启后活动任务为 interrupted，完成历史仍可查看。
- [ ] 删除会议清理数据库与受管音频；锁定文件导致删除失败时显示待清理。
- [ ] 导出 Markdown 为 UTF-8、章节稳定、内容完整；取消保存无错误/残留。
- [ ] Key 保存后只显示“已配置”；UI、普通日志和 SQLite 不出现值。
- [ ] 会议原文不出现在普通日志、错误提示、Toast、测试报告或截图。
- [ ] 键盘可完成核心流程；Escape 不直接取消活动任务或删除会议。
- [ ] 200% 缩放、高对比度、减少动画下核心控件可见可用。
- [ ] release 安装包在普通用户 Windows 环境安装、启动、卸载；记录签名/SmartScreen 事实。

## 10. Mock 端到端基准流程

建议 Phase 3 建立一条确定性测试：

1. 创建隔离的临时 app-data 和测试 Credential Store adapter，不接触用户真实数据。
2. 以相对路径或显式 `TEST_AUDIO_PATH` 引用真实音频资产；只校验存在、非空和可识别容器。
3. 创建一个处理任务，mock ASR 返回固定非敏感 transcript，mock minutes 返回 Schema-valid fixture。
4. 验证状态顺序和每阶段持久化；`completed` 只能在 SQLite 事务完成后出现。
5. 通过与 UI 相同的 repository/IPC 查询会议详情与搜索结果。
6. 导出到测试临时目录，按 UTF-8 重新读取并验证章节顺序与 fixture 内容。
7. 扫描日志、IPC 记录、测试报告和临时目录清单：无 sentinel secret、真实音频原文、Authorization 或绝对源路径。
8. 删除会议，验证受管副本/DB/索引清理，确认外部源音频仍存在。

该测试不证明真实 ASR 能识别音频；它证明本地 artifact 到 mock 转写、纪要、持久化、展示契约和导出的完整集成。

## 11. 取消、重试与时间测试方法

- 使用虚拟时钟验证 backoff、jitter 范围、Retry-After、overall deadline 和保留期限，避免真实长时间等待。
- mock transport 为每个 attempt 记录安全的 operation/attempt ID、阶段和 outcome，不记录 body/正文/路径。
- 每个取消测试设置有限完成时间；CancellationToken 必须传播到 semaphore、文件读取、上传、响应读取、退避、poll 和生成。
- 对 late response 使用同步屏障，确保取消先发生，再释放“成功响应”，验证数据库和事件均不接收成功。
- 对重复提交使用并发 barrier，同时发送同一 dedupe key，断言只存在一个活动 operation。
- 对 `timeout_after_send` 断言 attempt 数为 1，除非 adapter fixture 明确提供已验证幂等能力。

## 12. 缺陷分级与发布规则

| 级别 | 示例 | 发布规则 |
| --- | --- | --- |
| Blocker/Critical | Key/原文泄露；任意文件删除；取消后写入完成；数据库静默丢失；安装包无法启动 | 必须修复并复测；不得发布 |
| High | WAV/MP3/M4A/MP4/MOV 导入或 preflight 主路径失败；外部源被修改；401 无限重试；重复提交；历史重启不可读；Schema 绕过 | MVP 不得宣布完成 |
| Medium | 单个错误状态不清晰；删除残留可恢复但提示不足；特定文件来源/同步目录兼容问题 | 评估修复；若接受必须写已知限制 |
| Low | 非核心文案、轻微视觉偏差 | 可排期，不影响核心隐私/正确性 |

任何安全测试发现 sentinel 或真实 Key/正文泄露时：立即停止相关测试、删除不安全产物、通知 Lead；真实 Key 按事件流程轮换。不要在缺陷单中粘贴泄露值。

## 13. 测试证据模板

每次阶段验收使用以下最小记录：

```text
Build/Commit:
Windows build:
Node/pnpm/Rust versions:
Command or manual case ID:
Start/finish time:
Exit code / PASS / FAIL / BLOCKED:
Tests passed/failed/skipped:
Sanitized evidence path:
Issue IDs:
Notes and unverified areas:
```

证据文件不得包含真实密钥、Authorization、正文、音频或内部 endpoint。截图前检查窗口、Toast、终端和路径中是否存在敏感内容。

## 14. Phase 2/5 退出结论

类型检查、前端单元测试、Rust 单元测试、format、clippy、前端生产构建、直接 Rust mock 集成和最终 Windows NSIS 构建均已通过。命令缺失、裸路径入口、空 Key、URL、重复任务、取消落库和重启死锁已在代码层修复。真实 Tauri client 闭环、Credential Manager 实测、磁盘 SQLite/WAL 重启、锁定 staging cleanup、原子导出、真实 Provider 和 Windows 安装权限仍未通过。Lead 必须关闭第 2.4 节和安全审查中的剩余 High/Critical 项后，才能宣布 mock MVP、真实 API 或企业发布完成。
