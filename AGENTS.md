Project
这是一个只处理用户导入的离线音频文件、生成语音转写和 AI 会议纪要的 Windows 桌面工具。不采集麦克风或系统音频。

Engineering Rules
默认使用中文文档，代码和变量使用英文。
所有 API Provider 必须通过接口抽象，不允许写死供应商。
禁止硬编码 API Key、Token、Cookie 和内部地址。
禁止把完整会议原文写入普通日志。
所有网络请求必须有超时、错误分类和取消机制。
所有核心流程必须有 mock provider。
重要数据结构必须有 TypeScript 类型或 JSON Schema。
修改前先查看仓库结构和已有实现。
不要修改其他 Agent 正在负责的目录。
新增功能必须补测试。
不允许用伪造的测试结果作为完成证明。
Required Validation
完成任务前至少运行：

类型检查
单元测试
集成测试
构建命令
并在最终报告中写明真实执行结果

Security Incident Handling
测试密钥只能通过环境变量或 Windows 系统凭据管理器注入，禁止写入仓库文件。
如果发现明文密钥，立即停止使用、从工作区移除，并通知负责人在供应商控制台轮换。
不得把密钥值复制到日志、测试报告、文档、截图或 Agent 消息中。

Test Assets
仓库根目录可能包含本地测试录音。测试代码必须使用相对路径或显式配置，不得把录音复制进日志、测试快照或提交产物。

Windows Development Baseline
目标平台为 Windows 11 x64，桌面壳使用 Tauri 2。
音频入口仅支持用户选择或批量导入的离线文件；不得新增麦克风、系统音频或实时录音能力。
Node 依赖默认使用 pnpm；确定项目骨架后在 package.json 中固定 packageManager 版本。
Rust 使用 stable x86_64-pc-windows-msvc 工具链。
所有脚本和文档命令必须提供 PowerShell 兼容写法。

Phase Gates
Phase 0 只允许环境检查、技术调查、接口设计和文档，不创建大规模业务代码。
每个阶段必须经 Lead Orchestrator 汇总验证后，才能进入下一阶段。
真实 Provider 字段未知时，只定义适配器边界和 mock，不得推测响应结构。

Agent Ownership
Lead Orchestrator 负责根目录配置、跨模块类型统一、集成和阶段验收。
Agent 1 负责 docs/architecture.md 和 docs/mvp.md，不修改核心业务代码。
Agent 2 负责 frontend/src 及其前端测试。
Agent 3 负责 src-tauri/src/ingest、离线音频文件校验及其测试。
Agent 4 负责 Provider 适配层、mock provider 和 docs/api-contract.md。
Agent 5 负责会议纪要 Schema、Prompt、样例与解析测试；具体目录由 architecture.md 固化。
Agent 6 负责 docs/security-review.md 和 docs/test-plan.md；除修复明确安全缺陷外不修改业务代码。
不同 Agent 不得同时修改同一文件；需要跨目录变更时先通知 Lead Orchestrator。

Agent Completion Report
每个 Agent 完成后必须报告：修改文件、实现功能、运行命令、真实测试结果、已知问题、阻塞项、其他 Agent 需要遵守的接口。
