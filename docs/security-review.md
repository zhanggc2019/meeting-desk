# 安全与隐私审查

> 状态：Phase 2/5 实现复审，已完成代码静态审查、自动测试与 Windows NSIS 构建；尚未完成桌面端到端和安装实测
> 日期：2026-07-17
> 范围：Windows 11 x64、Tauri 2 桌面端、离线音视频文件导入、SQLite、本地文件、云端 ASR/LLM Provider、GitHub Release 自动更新
> 关联文档：[技术架构](./architecture.md)、[MVP 定义](./mvp.md)、[API 契约](./api-contract.md)、[测试计划](./test-plan.md)

## 1. 结论

当前实现已具备部分安全基础：Provider 抽象、redirect-disabled HTTP client、请求/响应上限、取消与重放安全策略、Windows Credential Manager、SQLite 参数化访问、WAV/MP3/M4A/MP4/MOV 预检、流式哈希、`.part` 精确清理、CSP 和最小插件 capability 均已落地。MP4/MOV 必须包含一条受支持音轨，应用不会调用外部 FFmpeg。自动更新固定到本仓库 GitHub Release endpoint，并强制验证 Tauri updater 签名；发布私钥不在仓库中。代码中未发现 `console`、`println`、`tracing` 等普通日志调用；会议文本由 React 文本节点渲染，没有 `dangerouslySetInnerHTML`。

当前**尚不能宣布安全或企业发布完成**。本轮已关闭命令缺失和 WebView 裸路径入口，并修复空 Key、URL、重复任务、取消后写库以及重启状态死锁；mock 音频到 SQLite 的 Rust 集成用例已经跑通。仍未完成真实 Tauri IPC/UI 端到端、Windows Credential Manager 实测、磁盘 SQLite 重启、锁定 staging 的 `cleanup_pending`、安装/启动/卸载和真实 Provider 验证。SQLite/WAL 仍明文保存会议数据，安装包也未签名。

当前有一个阻塞级安全事项：负责人已报告曾在工作区发现一枚明文测试密钥，文件中的值现已移除。**删除文件中的值不能使已暴露凭据重新安全**；在供应商控制台完成吊销/轮换并确认旧值失效前，事项 `SEC-P0-001` 保持 `BLOCKED`。本文不会记录或复述该密钥值。

自动更新的公开分发 blocker 已关闭：仓库已改为 Public，匿名 `latest.json` 与安装包下载均验证为 HTTP 200；下载仍须通过内置公钥校验签名，代码不内置 GitHub Token。

## 2. Phase 2/5 复审证据边界

### 2.1 已执行并确认

- 重新读取 `AGENTS.md`，直接审查 `frontend/src/**`、`src-tauri/src/**`、`package.json`、`Cargo.toml`、`.env.example`、Tauri 配置和 capability，不依赖 Agent 总结。
- `pnpm typecheck` 退出码 0；`pnpm test` 退出码 0，3 个文件、19 个前端测试通过，其中自动更新和 Markdown 预览在 React StrictMode 下验证加载状态可终结。
- `cargo test --manifest-path .\src-tauri\Cargo.toml --all-targets --all-features` 退出码 0，最终 96 个 Rust 测试通过；其中仓库本地真实 MP3 导入测试、MP4/MOV 结构与无音轨拒绝测试通过，但没有转写或记录其正文。
- 仓库外生成的 1 秒 H.264 + AAC MP4 已通过 `ffprobe` 和定向真实 importer 测试；样例未提交，测试不读取语音正文。
- `cargo fmt --check`、`cargo clippy ... -D warnings` 和 `pnpm build` 均退出码 0；Vite 主 JS 约 245 kB，gzip 约 76 kB。
- `pnpm test:integration` 退出码 0，1 个 Rust 集成用例通过：合成 WAV 经真实 ingest、MockProvider、纪要校验、内存 SQLite 保存并清理 staging。该脚本不是 Tauri IPC/UI、磁盘 SQLite、重启或导出 E2E。
- `pnpm tauri:build` 和 `pnpm tauri:build:release` 均退出码 0；后者生成 4,273,464-byte NSIS、424-byte updater `.sig`，release exe 启动检查通过。Updater 私钥只位于仓库外受控目录和 GitHub Actions Secret。
- 扫描 144 个工作区文本文件，发现 2 个明确无效的测试 sentinel 文件，真实/高置信度供应商凭据命中 0；`.env.example` 的两个 Key 均为空；Git 跟踪音频文件数为 0。
- 当前已有远端基线提交，可执行有意义的 diff、跟踪文件和敏感信息审查。本轮 updater 私钥位于仓库外目录并写入 GitHub Actions Secret，未进入工作树。此前曾暴露的测试密钥仍无供应商侧轮换证据。
- release 产物和 bundle 目录未发现 `.env*`、MP3/WAV/M4A/MP4/MOV、SQLite/DB 或日志文件。安装器具备 Tauri updater 签名，但 Authenticode 状态仍为 `NotSigned`。
- `pnpm audit --prod --registry=https://registry.npmjs.org` 退出码 0，未报告已知生产依赖漏洞；默认镜像没有 audit endpoint，Rust `cargo-audit` 未安装，Rust 依赖漏洞审计未完成。

### 2.2 尚未验证

- 已暴露测试密钥是否已在供应商侧吊销/轮换，旧值是否确实失效。
- Windows Credential Manager 的真实写入、留空不替换、覆盖、删除、环境变量回退和重启行为；当前只有 mock UI 测试和代码审查。
- Tauri 主流程端到端：前端声明的 15 个命令均已注册；浏览器已实测配置引导和 Markdown 预览，但尚未用真实 WebView/IPC 自动或手工跑完整闭环。
- staging 成功文件已有显式释放、成功/取消终态清理和 best-effort 启动清理；尚未验证 Windows 锁定文件、持久化 `cleanup_pending`、多残留部分失败和卸载残留。
- 桌面裸路径拖放 command 已移除；桌面只接受系统文件对话框。浏览器 mock 的拖放不代表 Tauri 桌面拖放能力。
- Markdown 预览只接收 Rust 导出渲染器生成的文本，前端 `react-markdown` 启用 `skipHtml` 且未启用 raw HTML 插件；会议原文中的 HTML 不会作为 DOM 执行。
- SQLite/WAL/SHM 和 staging 目录实际 ACL、普通用户/其他用户读取、磁盘满、损坏库、安装/卸载残留。
- 真实 Provider、TLS/代理、自签 CA、请求 timeout、401/403/429/5xx、远端取消和真实 response 上限。
- NSIS 实际安装/启动/卸载、UAC/安装模式、WebView2 缺失、SmartScreen、非管理员用户和企业 EDR；当前只完成构建。

## 3. 数据分类与信任边界

| 数据 | 分类 | 允许位置 | 禁止位置 |
| --- | --- | --- | --- |
| ASR/LLM API Key、Token、Cookie | 机密凭据 | Windows Credential Manager；开发测试可短期从进程环境读取 | SQLite、前端 store、URL、日志、截图、测试 fixture、`.env.example`、Git |
| 原始音频/视频、临时媒体、导入副本 | 高敏感会议数据 | 受管 app-local-data 目录；用户明确选择的源文件位置 | WebView 内存、普通日志、测试快照、仓库、无界临时目录 |
| Tauri updater 私钥 | 发布根密钥 | GitHub Actions Secret；发布者受控凭据目录 | Git、构建日志、应用包、普通 CI artifact |
| 完整转写、segments、Prompt、纪要 | 高敏感会议数据 | Rust 受信任内存、SQLite、详情页按需内存、用户显式导出 | 普通日志、URL、浏览器存储、错误监控 breadcrumb、Provider metadata |
| 会议标题、参会人、文件名、绝对路径 | 敏感元数据 | SQLite、受控 UI、必要的系统文件对话框 | 普通日志、遥测、公开错误信息 |
| task/session/operation ID、阶段、耗时、HTTP status、安全错误码 | 低敏感诊断数据 | 普通日志、IPC 状态事件 | 与正文、凭据或可逆路径绑定的诊断 dump |
| Provider endpoint/model | 配置数据，内部地址时为敏感 | SQLite 非密钥配置、受控设置 UI | 未经批准的遥测；带 userinfo/query 的日志 |

主要信任边界：

1. **WebView/React：低权限。** 只消费中立 DTO 和安全错误，不直接读取文件、数据库、凭据或发送 Provider 请求。
2. **Tauri Rust Core：受信任。** 承载音频、SecretStore、Provider transport、SQLite、导出和删除；所有 IPC 输入都视为不可信。
3. **云端 Provider：外部且不可信。** 请求会合法包含音频或转写正文；响应、header、重定向和远端 request id 都必须校验后才可保留。
4. **本地文件系统与导出目标：部分可信。** 路径可能包含重解析点、UNC、同步目录或用户可控文件；不能拼接路径后直接覆盖。
5. **日志、崩溃报告和测试产物：低敏感通道。** 只能接收字段白名单数据。

## 4. 威胁模型

### 4.1 保护目标

- 防止 API Key 被前端、日志、数据库、安装包或 Git 暴露。
- 防止会议音频、原文和纪要被非预期写盘、上传、记录或导出。
- 防止不可信 UI/Provider 输入获得任意文件、网络、Shell 或数据库权限。
- 防止取消、重试和重复提交导致重复计费、远端任务继续运行或错误标记完成。
- 防止崩溃、断电、导入中断和删除失败留下被误认作完整数据的残留。
- 保证已保存记录、导出和删除状态与实际数据库/文件状态一致。

### 4.2 主要威胁与控制

| 威胁 | 场景 | 影响 | 必需控制 | 验证 |
| --- | --- | --- | --- | --- |
| 凭据泄露 | Key 进入日志、UI、SQLite、配置 dump、Git | 账户滥用、数据泄露、费用损失 | Credential Manager；只写不读回 UI；secret wrapper；日志白名单；仓库扫描；轮换流程 | `SEC-*`、`CFG-*` |
| 原文泄露 | HTTP/body trace、panic、测试 snapshot、文件名日志 | 企业会议内容泄露 | 禁止 body/header dump；安全错误映射；sentinel 扫描；前端不持久正文 | `LOG-*`、`IPC-*` |
| IPC 权限提升 | 恶意/受污染 WebView 调任意文件或网络命令 | 读取本地文件、上传数据 | 精确 Tauri command；参数校验；最小 capabilities；不加载远程页面 | `TAURI-*` |
| 路径穿越/重解析点 | artifact id 或导出路径被构造 | 覆盖/删除任意文件 | opaque artifact id；canonicalization；受管根目录检查；系统保存对话框；重解析点测试 | `FILE-*` |
| SSRF/凭据转发 | 用户配置恶意 endpoint、HTTP redirect 到新主机 | 内网探测、Key 外发 | HTTPS 默认；禁 userinfo/query；限制 redirect；跨源不转发认证；开发 HTTP 仅 localhost | `NET-*` |
| 恶意 Provider 响应 | 超大/畸形 JSON、HTML/Markdown 脚本 | OOM、XSS、状态污染 | 响应大小上限；Schema；纯文本渲染；CSP；不执行 HTML | `API-*`、`MIN-*` |
| 重试/取消竞态 | body 发出后 timeout 自动重放；取消后 late success | 重复计费、重复会议、隐私误报 | ReplaySafety；Operation/Attempt ID；CancellationToken；late result 丢弃；本地单飞 | `TASK-*` |
| 本地残留 | 未完成 staging、SQLite WAL、失败导出、删除失败 | 后续用户或备份读到敏感数据 | 生命周期策略；启动清理；cleanup pending；事务；明确残留提示 | `RET-*`、`DB-*` |
| SQLite 篡改/损坏 | 非预期退出、磁盘满、手工修改 | 历史丢失、假完成 | migrations；事务；完整性检查；`completed` 保存后置；安全恢复 | `DB-*` |
| Prompt 注入 | 转写包含“忽略规则”等指令 | 纪要偏离 Schema、泄露额外内容 | 将 transcript 明确作为不可信数据；固定系统指令；Schema 校验；不提供工具 | `MIN-*` |
| 供应链/打包 | 依赖被篡改、过宽 capability、调试功能发布 | 任意代码执行、权限扩大 | 锁文件；审计；本地 CLI；release 配置审查；安装包签名计划 | `PKG-*` |

本项目不是对本机管理员、已控制当前 Windows 用户会话的恶意软件提供强隔离。Windows 用户目录 ACL 只能防止其他普通账户直接读取，不能替代磁盘加密、终端防护和企业数据治理。

## 5. 凭据与配置要求

### 5.1 生产凭据

- 生产 Key 只保存到 Windows Credential Manager。SQLite 只保存 opaque `credentialRef` 和 `secretConfigured` 状态。
- 前端允许“填写/替换/删除”，不提供读取或显示原值的命令。`get_public_settings` 只能返回布尔状态。
- Rust 中的 secret 类型不得派生会输出内容的 `Debug`/`Display`；HTTP 层只在发请求的最后边界注入凭据，生命周期结束后尽力清零内存。
- 删除 Provider 配置时必须明确区分“删除非密钥配置”和“删除凭据”；清理全部会议数据不能暗中保留或暗中删除凭据。
- 凭据写入失败不得回退为明文 SQLite/JSON；应返回安全错误并保持未配置状态。
- Auth header 名只允许由经审查 adapter 选择，UI 不允许配置任意 secret header。

### 5.2 开发与测试凭据

- 只能通过进程环境或 Credential Manager 注入；`.env.example` 只能给空值/说明，真实 `.env` 必须被忽略。
- PowerShell 脚本不得 `Write-Output`、`echo`、插值或捕获环境变量值；禁止 `curl -v`、HTTP body trace 和会显示环境的诊断 dump。
- 测试 secret 使用明确无效且不模仿真实供应商格式的 sentinel，不使用被报告已暴露的旧 Key。
- 真实 Provider 验证使用最小非敏感音频，验证报告只记录 HTTP status、字段结构、长度和安全错误，不记录正文。

### 5.3 已报告密钥事件

处置顺序：

1. 立即停止使用该测试 Key，确认工作区现值已移除。
2. 由具备供应商控制台权限的负责人吊销/轮换，并确认旧 Key 不再可用。
3. 检查供应商审计/用量记录是否有异常；不得把明细中的 Key 或会议内容复制到项目文档。
4. Git 初始化后只扫描新仓库历史不能覆盖此前外部副本；还需检查聊天附件、终端历史、编辑器历史、云同步和备份等传播面。
5. 以不含值的事件记录关闭 `SEC-P0-001`，注明轮换时间、执行角色和确认方式。

## 6. 日志、错误和诊断

- 使用 allowlist 结构化日志，只允许 task/session/operation/attempt ID、阶段、耗时、adapter 版本、HTTP status、稳定错误码和有限音频元数据。
- 文件名、绝对路径、标题、参会人、音频哈希也不进入普通日志；诊断使用 artifact ID。
- 永不记录 request/response body、multipart、header 全量、endpoint query、音频 bytes、transcript、minutes、prompt 或底层 request 的 `Debug` 输出。
- `Authorization`、`Proxy-Authorization`、Cookie 及名称含 key/token/secret/password/credential 的字段完全省略；`[REDACTED]` 只用于必须证明遮蔽的测试场景。
- Rust panic hook、React ErrorBoundary、unhandled rejection、数据库错误和 Tauri command error 必须先映射成安全错误；底层 cause 只允许进入受控开发诊断且仍不得包含敏感对象。
- release 默认关闭 DevTools 和 verbose HTTP/SQL tracing。不集成第三方崩溃/分析 SDK，除非另行完成数据处理评估与 opt-in 设计。
- 自动泄露测试必须扫描日志、stderr/stdout 捕获、IPC fixture、snapshot、JUnit/HTML 报告和导出失败信息。

## 7. 离线音频导入与临时文件

- 只接受用户通过系统文件对话框明确选择的单个或批量 WAV、MP3、M4A、MP4、MOV 文件；扩展名只是提示，preflight 必须检查零字节、容器/MIME、视频音轨、可解析性、字节数、时长及 Provider 已验证的格式/大小能力。
- 外部源文件始终视为只读：应用不得修改、重命名、移动或删除源文件。处理、取消、会议删除和清理全部数据都不能影响外部源。
- 后端把文件对话框结果转换为 opaque artifact ID；前端只接收显示名和安全元数据，不接收可用于任意文件访问的绝对路径。
- preflight 和上传之间必须防止检查后替换风险。至少重新核对文件身份、大小和修改时间；需要完整性或去重时以流式方式计算 SHA-256，并在真正上传前复核。哈希属于敏感元数据，不进入普通日志。
- 不把整个音频读入内存或 WebView。preflight、hash、受管 staging copy 和上传都采用有界流式 I/O，并共同响应取消 token。
- 如处理需要 staging 副本，只能写入 app-local-data 下的受管目录，使用不可预测文件名、当前用户最小 ACL 和明确的 `staging` 状态。复制完整、flush/close 并校验后才能发布为可上传 artifact。
- staging 创建失败、磁盘满、源文件读取中断或程序崩溃时不得创建可处理任务；残留登记 `cleanup_pending`。应用重启只清理由数据库/manifest 证明属于本应用的残留，禁止用宽泛 glob 删除文件。
- 批量导入按文件独立预检和建任务。一个文件零字节、损坏、超大或格式不支持，不得阻塞其他合法文件，也不得把整个批次标记为全部成功。
- 用户取消上传、转写或总结时，停止后续本地读取和任务推进；不再需要的 staging 副本按策略删除。删除失败进入 `cleanup_pending`，UI 不得宣称完全清理。
- 不能依赖开发机 FFmpeg 作为客户端隐式运行依赖。解析/解码器对损坏文件、异常元数据和压缩炸弹式输入必须设置字节、时长、CPU 与内存上限。
- 隐私默认建议：任务结果事务性保存后删除 staging 与失败中间文件。外部源的生命周期始终由用户控制；如产品允许保留受管副本，必须提供明确期限和立即清理入口。

## 8. SQLite、本地搜索与恢复

- SQLite 位于 app-local-data，仅 Rust repository 可访问；不放在仓库或 WebView 可读目录。
- 所有 schema 变更使用版本化 migration；任务状态、transcript、valid minutes 与 meeting 保存必须事务化，`completed` 是最后写入的状态。
- API Key 不进入 SQLite。transcript、minutes、标题和 FTS 索引均视为明文敏感数据。
- WAL/SHM、临时表、FTS 索引和备份同样可能包含正文；清理、迁移、复制和测试都必须把它们纳入范围。
- Phase 0 选择“Windows 用户 ACL + 本机数据”而非数据库加密。这是明确的剩余风险：同一用户会话、管理员、磁盘镜像、备份/同步和恶意软件仍可能读取。企业发布前需决定是否要求 BitLocker/设备基线或数据库加密。
- 不声称 SQLite `secure_delete` 或文件覆盖能在 SSD、NTFS 快照、云同步或备份上不可恢复地擦除。UI 文案应为“从本应用本地数据中删除”，并说明导出、备份和云端 Provider 副本不受其控制。
- 数据库损坏、磁盘满和 migration 失败必须失败关闭，保留原文件以便受控恢复，不得创建空库后静默显示“没有历史”。
- 搜索只在本地执行，不把查询文本发送云端；查询参数化并限制结果数/资源占用。

## 9. 导出、删除与保留

### 9.1 Markdown 导出

- 仅通过系统保存对话框取得用户选定路径；取消不是错误。
- 不使用会议标题直接拼接任意路径，不调用 Shell 解释用户内容，不自动打开导出文件。
- 内容按 UTF-8 写入，模型文本按数据处理；应用内预览不执行原始 HTML/脚本。若后续支持 Markdown HTML 渲染，必须采用严格 sanitizer。
- 导出采用同目录临时文件 + 原子替换或等价策略；失败后清理临时文件并保留已有目标，不留下半写文件。
- UI 应提示导出文件不再受应用保留/删除策略控制，尤其是 UNC、OneDrive 或企业同步目录。

### 9.2 删除语义

- 删除单条会议要覆盖 meeting、transcript、minutes、task/attempt、受管 artifact 及搜索索引；用户外部导入源文件和用户导出文件永不自动删除。
- 数据库删除与文件删除无法形成同一原子事务。建议先写删除意图/待清理状态，再删除受管文件，最后事务性删除/脱敏业务记录；任一步失败都保留可重试的安全状态。
- “清理全部本地数据”需要二次确认，且必须单独询问是否删除 Provider 凭据。
- 取消任务应停止本地后续处理；若 Provider 不支持远端取消，UI 必须说明远端可能继续执行/计费，不能宣称云端数据已删除。
- 保留期限、成功后音频策略、失败任务保留期和日志轮转上限必须在 Phase 1 配置基线中固化。发布前这些值未确定则为 `BLOCKED`。

## 10. Tauri 2 安全基线

- capabilities 只绑定主窗口和必要命令；不授予通用 shell、任意文件系统、任意 HTTP、进程启动或全局路径权限。
- 前端通过自定义命令使用 audio/task/meeting/settings 能力。命令参数需要长度、枚举、ID、状态转换和路径归属校验，不能信任 TypeScript 类型。
- 不加载远程页面，不向远程 origin 开放 Tauri API；导航、`window.open`、下载和自定义协议均应限制。
- CSP 默认拒绝远程脚本和内联执行；仅允许构建产物需要的最小资源。会议文本使用 React 文本节点，不使用未净化 `dangerouslySetInnerHTML`。
- release 关闭 DevTools，移除调试命令、测试 mock 控制台和会返回 secret/path 的内部 API。
- 文件对话框返回值仍不可信；artifact/delete 命令只接受 opaque ID，导出命令只接受 meeting ID 并在后端发起保存对话框。
- Tauri config、capability JSON、依赖和 updater/签名设置必须进入 Phase 6 独立审查。代码签名不是当前 MVP 功能目标，但无签名安装包不得被描述为适合正式企业广泛分发。

## 11. Provider 请求安全

- 生产 endpoint 默认只允许 HTTPS，拒绝 userinfo 和 fragment，query 默认不接受；开发 HTTP 仅显式允许 localhost mock。
- 关闭自动跨源认证重定向；如允许重定向，必须限制 scheme/host 并在跨源时删除敏感 header。TLS 校验不得提供“忽略证书错误”的普通设置。
- 内部 Provider、自签 CA 和企业代理属于后续受控配置，不通过全局关闭 TLS 校验解决。
- 每个请求有 connect/request/overall timeout、响应大小限制、并发限制和 CancellationToken；读取文件和响应均采用有界流式处理。
- 重试必须同时满足错误临时、ReplaySafety、attempt 上限、总 deadline 和未取消。body 发送后的未知结果无已验证幂等时不得自动重放。
- 429 尊重有上限的 `Retry-After` 并共享 cooldown；401/403/结构性 4xx 不自动重试。
- Provider 返回的远端 request id、错误文本、URL 和模型 metadata 在进入数据库/IPC 前脱敏与长度限制；原始 body 永不进入普通日志。
- ASR 音频和 LLM transcript 是用户主动配置 Provider 后的预期数据出境。首次真实配置应清楚提示数据将发送至该 endpoint；应用不能声称云端会自动删除。

## 12. 安全发现与状态

| ID | 严重度 | 状态 | 发现 | 必需动作/关闭条件 | 责任方 |
| --- | --- | --- | --- | --- | --- |
| `SEC-P0-001` | Critical | **BLOCKED** | 曾发现明文测试密钥，当前值已从工作区移除，但未提供供应商侧轮换证据 | 吊销/轮换；确认旧值失效；检查异常用量；记录不含值的关闭证据 | Lead + 凭据所有者 |
| `SEC-P25-001` | High | **FIXED-CODE / VERIFY** | 保存层现会忽略空白 Key，显式删除仍走独立命令，和“留空不替换”一致 | 增加 Credential Manager/IPC 回归：首次保存、空白保持、替换、显式删除、环境变量回退和重启 | Lead + UI |
| `SEC-P25-002` | High | **CLOSED** | `register_dropped_audio_files` 和原生路径 invoke 已完全移除；桌面端只从 Rust 系统文件对话框取得路径 | 保持命令面无裸 `PathBuf`；如恢复拖放，必须在可信侧绑定授权后复审 | Lead + Ingest |
| `SEC-P25-003` | High | **MITIGATED / OPEN** | 已有候选显式 release、成功/取消终态清理、失败清理重试和 best-effort 启动清理；测试证明受管副本删除而源文件不变 | 仍需持久化 `cleanup_pending`，覆盖 Windows 锁定文件、多残留部分失败、崩溃和卸载；当前启动清理失败只在下次启动重试 | Lead + Ingest |
| `SEC-P25-004` | High | **FIXED-CODE / VERIFY** | 前端 15 个命令已全部注册；额外后端命令为删除会议/凭据。Mock 集成用例已跑通 ingest 到保存；Markdown 预览与导出共用后端渲染器 | 使用真实 Tauri client 跑 UI/IPC、磁盘 SQLite、重选、搜索、详情、预览和导出 E2E；不能用直接 Rust 函数测试代替 | Lead |
| `SEC-P25-005` | High | **FIXED-CODE / VERIFY** | 设置保存层现拒绝 endpoint 的 userinfo、password、query 和 fragment，并保持 HTTPS/loopback HTTP 策略 | 补四类恶意 URL 参数化测试，并验证拒绝前 SQLite 未写入 | Lead + Provider |
| `SEC-P25-006` | Medium | Open | 含 Key 的 `ProviderSettingsInput`/外层输入派生 `Debug`，Key 以普通 `String` 经 React state 和 IPC 传输；当前无日志调用，但扩大了未来误打日志风险 | 移除敏感输入 `Debug`；使用受控 secret wrapper/最短生命周期；release 禁 DevTools；增加 sentinel IPC/错误扫描 | Lead + UI |
| `SEC-P25-007` | Medium | Open | Provider `Transcript`/`MinutesCandidate` 已自定义脱敏 `Debug`；但 domain `MeetingDetail`、`PersistedMeetingInput` 和响应结构仍可直接 Debug 输出正文 | 为剩余敏感 DTO 自定义 redacted Debug 或移除 Debug；建立日志 allowlist 测试 | Lead |
| `SEC-P25-008` | Medium | Open | SQLite/WAL 明文保存 transcript、segments、minutes 和敏感文件名；未验证 app-local-data/staging ACL，也没有数据库加密 | 验证 Windows ACL/普通用户隔离；明确 BitLocker/加密基线与剩余风险；覆盖 WAL/备份/删除说明 | Lead + 安全 |
| `SEC-P25-009` | Medium | Open | Provider remote request id 从可配置 allowlist header 原样进入 metadata，没有长度/字符或敏感模式限制 | 限长、限制字符并禁止 credential-like 值；不进入普通日志；补恶意 header 测试 | Provider |
| `SEC-P25-010` | Medium | **CLOSED** | 浏览器测试客户端、UI accept 和真实 importer 已统一为 WAV/MP3/M4A/MP4/MOV；真实 importer 继续执行容器和音轨校验 | 保持能力定义单一来源；E2E 仍必须走 Tauri ingest | UI + Ingest |
| `SEC-P25-011` | Medium | **PARTIAL** | `test:integration` 已存在且通过 1 个直接 Rust 闭环；仍只使用合成 WAV 和内存 SQLite，未覆盖桌面 IPC、磁盘重启、取消竞态、锁定清理、Credential Manager 和导出 | 扩展为独立磁盘/IPC 集成套件并纳入统一命令/CI | Lead + QA |
| `SEC-P25-012` | Medium | Open | NSIS 和应用均未签名，未显式配置/实测安装模式、UAC、SmartScreen 或卸载残留 | 正式企业分发前签名；记录 current-user/per-machine 策略并实机安装/卸载 | Lead + QA |
| `SEC-P25-013` | Low | **CLOSED** | 最终 `cargo fmt --check` 和全量 clippy `-D warnings` 均通过 | 继续纳入统一验证 | Lead |
| `SEC-P25-014` | High | **FIXED-CODE / VERIFY** | `task_gate` 串行化取消与所有状态写入；先持久化 `CancelRequested` 再发信号，终态保存与令牌移除同锁完成；新增预取消流水线终态回归 | 增加真实同步屏障并发测试，证明取消先于 late success 时会议和 completed 状态均不残留 | Lead |
| `SEC-P25-015` | High | **FIXED-CODE / VERIFY** | `task_gate` 串行化创建和重试；活动 token 拒绝覆盖；`max_attempts` 已强制执行并有上限/重启回归 | 增加并发 barrier/多线程单飞测试；验证不同 artifact 不被错误阻塞 | Lead |
| `SEC-P25-016` | Medium | Open | Markdown 直接 `std::fs::write` 用户目标；磁盘满/崩溃时可能半覆盖已有文件 | 使用同目录临时文件、flush 和原子替换；补已有目标与写失败测试 | Lead |
| `SEC-P0-002` | Medium | **CLOSED** | 已有远端基线提交；本轮完成 diff、ignore、tracked file 和高置信度 secret scan | 每次发布前持续执行相同扫描 | Lead |
| `SEC-P0-007` | Medium | Open | 删除无法保证清除备份、SSD 残留或 Provider 副本 | UI/README 明确删除边界；实现 cleanup pending 与重试 | Lead |

## 13. 阶段安全门槛

### 进入真实 Provider 前

- 关闭 `SEC-P0-001`，并完成 `SEC-P25-001`、`SEC-P25-005` 的 Credential Manager/持久化回归；真实 Key 只通过新 Credential Manager 项或进程环境注入。
- 对已落地的任务编排证明 timeout、401、429、5xx、取消 late success、并发单飞和 unknown outcome 不安全重放。

### 宣布 mock MVP 前

- 关闭 `SEC-P25-003`、`SEC-P25-004`、`SEC-P25-011`；用真实音频 artifact 和 Tauri client 跑通 mock 全流程。当前直接 Rust 集成用例只算部分证据。
- 验证重启恢复、批量部分失败、staging 清理、SQLite 磁盘持久化和 Markdown 导出。

### 发布前

- 完成 Windows ACL、安装/卸载、非管理员运行、WebView2、SmartScreen/签名和 Rust 依赖漏洞审计。
- 建立 Git 基线后由独立 Reviewer 直接检查 diff、capabilities、secret flow、请求重试/取消和测试输出。

## 14. Phase 2/5 审查结论

基础模块的单元测试、格式、clippy、前端构建与 mock Rust 集成用例均已通过；HTTP 层的重定向禁用、超时/响应上限、取消和重试分类值得保留。裸路径授权、空 Key、URL、重复任务、取消后写库及重启死锁已在代码层修复。当前主要缺口转为验证深度和发布控制：真实 Tauri/IPC E2E、Credential Manager、锁定文件 cleanup、磁盘 SQLite、原子导出、真实 Provider、安装实测和签名仍未关闭；此前密钥事件也仍待供应商侧轮换证据。因此可继续进行无真实 Key 的 mock 集成，但不能宣称真实 Provider 或企业发布安全完成。
