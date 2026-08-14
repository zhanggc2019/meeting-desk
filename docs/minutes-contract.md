# 会议纪要 Schema、Prompt 与校验契约

> 状态：Phase 0 契约基线
> 日期：2026-07-17
> 适用范围：Agent 5 在 Phase 2 负责的 `src-tauri/src/minutes/**`、`shared/schemas/**`、`shared/fixtures/minutes/**`
> 实现状态：本文只定义契约和测试计划；尚未创建可执行 Schema、Prompt、fixture、解析器或测试，也未调用真实模型。

本文与 [技术架构](./architecture.md)、[MVP 定义](./mvp.md) 和 [Provider 契约](./api-contract.md) 共同构成会议纪要模块的输入、输出和安全边界。真实 LLM 是否支持 JSON Schema、最大输入长度及具体请求字段均保持未验证；Provider adapter 不能复制或改写本文定义的业务 Schema。

## 1. 结论与边界

Phase 2 应实现一个唯一、版本化的 `MeetingMinutes` JSON Schema，并由 Rust 纪要模块持有 parser、Schema validator、semantic validator 和模板注册表。全部内置模板共享同一输出 Schema，只改变内容侧重点，不产生多套不兼容 DTO。

处理边界如下：

```text
Transcript + MinutesRequest.meetingContext
  -> 输入预检与质量标记
  -> 单段生成，或长文本分块提取/合并
  -> 严格 JSON 解析
  -> MeetingMinutes JSON Schema 校验
  -> 确定性语义/来源校验
  -> MeetingMinutesEnvelope
```

强制规则：

- 空白 transcript 在调用 LLM 前返回 `empty_transcript`；不能生成一份“看似正常”的空会议纪要。
- 完整 transcript 单独存储，不嵌入 `MeetingMinutes`，不写普通日志、错误、快照或测试报告。
- 模型输出是未受信任数据。只有同时通过 JSON Schema 和语义校验的结果才能进入 `MeetingMinutesEnvelope`。
- **不得推断参会人姓名、会议时间、负责人或截止日期。** 下文对这四类字段采用比其他摘要字段更严格的来源规则。
- speaker label 只是匿名声纹/分段标签，永远不能自动映射为真实姓名。
- transcript 中出现的指令、JSON、Markdown 或“忽略之前要求”等文本都属于会议内容，不能改变系统 Prompt、Schema 或模板规则。

## 2. Schema 版本与唯一规范来源

### 2.1 版本规则

- 首版 Schema 版本定为 `1.0.0`，使用 JSON Schema Draft 2020-12。
- Phase 2 规范文件建议为 `shared/schemas/meeting-minutes/1.0.0.schema.json`，`$id` 使用不含内部地址的稳定 URN，例如 `urn:funasr-demo:meeting-minutes:1.0.0`。
- `MeetingMinutes.schemaVersion` 必须为常量 `"1.0.0"`。
- `MinutesRequest.outputSchemaVersion`、`MeetingMinutes.schemaVersion`、`MeetingMinutesEnvelope.schemaVersion` 和 `validation.schemaVersion` 必须完全相同。任何不一致均返回 `schema_version_mismatch`，不得自动猜测或迁移。
- 同一 major 版本中，新增 required 字段或改变既有字段语义属于 breaking change，必须提升 major。只增加说明或收紧不影响既有合法实例的校验可提升 patch。
- 历史数据按其原版本读取。迁移必须是显式、可测试的纯函数；不得在读取时静默覆盖原 JSON。

`templateId` 和 `templateVersion` 是 `MinutesRequest`、任务和持久化元数据，不让模型回显，也不放进 `MeetingMinutes` v1.0.0。可信编排层负责把模板版本与结果关联，避免模型伪造模板身份。

### 2.2 required / nullable 总策略

顶层和所有对象均采用：

- `additionalProperties: false`；
- 所有定义过的属性都列入 `required`；
- 未知的单值事实用 JSON `null`；
- 没有条目的集合用 `[]`，集合本身不允许 `null`；
- 禁止用 `"未知"`、`"无"`、`"待定"`、`"N/A"` 或空白字符串替代 `null`/`[]`；
- 所有非空字符串 trim 后必须至少含一个非空白字符；
- JSON 字段名和枚举值使用英文，面向用户的内容默认使用 transcript 的主要语言。

这样 UI 可以稳定渲染每一个章节，同时能区分“字段缺失/模型漏写”和“信息确实未知”。JSON Schema 负责 required、类型、格式、长度和额外字段；来源真实性、跨字段关系和证据引用由语义校验器负责。

## 3. `MeetingMinutes` v1.0.0 规范形状

以下 IDL 是 Phase 2 JSON Schema 的规范性语义，不是当前已实现代码：

```ts
interface MeetingMinutesV1 {
  schemaVersion: "1.0.0";
  title: string | null;
  titleSource: "context" | "generated" | "unknown";
  meetingTime: {
    startAt: string | null; // RFC 3339 date-time，仅可复制 meetingContext.knownStartAt
    endAt: string | null;   // RFC 3339 date-time，仅可复制 meetingContext.knownEndAt
  };
  participants: string[];
  summary: string | null;
  topics: Topic[];
  conclusions: SupportedStatement[];
  decisions: SupportedStatement[];
  actionItems: ActionItem[];
  risksAndIssues: RiskOrIssue[];
}

interface Topic {
  title: string;
  summary: string | null;
  evidenceSegmentIds: string[];
}

interface SupportedStatement {
  content: string;
  evidenceSegmentIds: string[];
}

interface ActionItem {
  description: string;
  owner: string | null;
  dueDateText: string | null;
  dueDate: string | null; // YYYY-MM-DD；只由可信代码从完整、明确日期规范化
  evidenceSegmentIds: string[];
}

interface RiskOrIssue {
  kind: "risk" | "issue";
  description: string;
  impact: string | null;
  mitigation: string | null;
  evidenceSegmentIds: string[];
}
```

### 3.1 字段语义

| 字段 | 语义和来源 | 空值规则 |
| --- | --- | --- |
| `schemaVersion` | 业务 payload 的精确 Schema 版本 | 必须为 `1.0.0` |
| `title` | 有 `knownTitle` 时原样使用；否则可从明确主题生成简短描述性标题 | 内容不足时 `null` |
| `titleSource` | `context` 表示与 `knownTitle` 完全一致；`generated` 表示模型基于内容生成；`unknown` 表示标题为 `null` | 与 `title` 联合校验 |
| `meetingTime` | 只接受 `meetingContext.knownStartAt/knownEndAt`；录音文件时间、转写内容、“今天/刚才”等均不得作为会议时间 | 未提供的端点为 `null` |
| `participants` | 只接受 `meetingContext.knownParticipants` 中的姓名；保持首次出现顺序并去重 | 未提供时 `[]` |
| `summary` | 只概括得到转写证据支持的核心内容，不加入常识、建议或未表达动机 | 内容不足时 `null` |
| `topics` | 实际讨论的主题；`summary` 是该主题的有证据概括 | 无可靠议题时 `[]` |
| `conclusions` | 会议中已经形成的结论；讨论、建议和猜测不能升级为结论 | 无明确结论时 `[]` |
| `decisions` | 明确确认、同意或拍板的决定；提议、选项、待确认事项不属于决策 | 无明确决策时 `[]` |
| `actionItems` | 明确要求执行的事项。描述可提炼，负责人和截止日期受特殊来源规则限制 | 无明确待办时 `[]` |
| `risksAndIssues` | 风险、阻塞、异议、未决问题和需要人工核对的重要低置信度内容 | 无项目时 `[]` |

### 3.2 子字段与证据规则

- `evidenceSegmentIds` 只能引用当前 `Transcript.segments[].id`，必须去重并保持原始时间/数组顺序。
- Provider 没有 segments 时 `evidenceSegmentIds` 合法值为 `[]`；不得为制造证据而虚构 segment、时间戳或 ID。
- 有 segments 时，结论、决策、待办、风险/问题原则上必须至少有一个有效证据 ID。无法定位证据的条目不得输出为确定事实。
- `summary` 与生成标题是跨段综合字段，不要求列出证据数组；它们仍不得引入条目字段之外的新事实。
- `owner` 只有在原文把明确的人名/团队名和该待办直接绑定时才可填入，并必须是对应证据文本中的原样子串。`我`、`他/她/他们`、`我们`、`这边`、speaker label 或根据发言顺序猜出的姓名一律写 `null`。
- `dueDateText` 只保存与待办直接绑定的原文期限短语，例如“下周五”或“7 月 20 日”，且必须是对应证据文本中的原样子串。没有明确期限时为 `null`。
- 模型不得填写 `dueDate`。可信后处理只在 `dueDateText` 本身包含完整、无歧义的公历年月日时转换为 `YYYY-MM-DD`；“明天”“月底”“7 月 20 日”这类缺少绝对信息的文本保留在 `dueDateText`，`dueDate` 必须为 `null`。
- `impact` 和 `mitigation` 只在原文明确讨论时填写；不能根据风险描述自行补充影响或建议。
- 对象数组的完全重复项必须去重；语义近似去重不能丢掉冲突信息。

### 3.3 结构限制建议

Phase 2 Schema 应对未受信任输出设置边界：标题建议不超过 200 字符，单个正文项不超过 2,000 字符，摘要不超过 5,000 字符，单个数组不超过 100 项，单个 evidence 数组不超过 100 个 ID。最终数值由 Lead 在 Phase 2 固化并写测试；一经进入 `1.0.0` 不得因 Provider 差异私自改变。HTTP `maxResponseBytes` 仍是更外层的硬限制，Schema 长度限制不能替代传输限额。

## 4. 确定性语义校验

JSON Schema 无法证明内容没有幻觉。Phase 2 必须在 Schema 校验后执行语义校验，至少覆盖：

1. `meetingTime.startAt/endAt` 分别严格等于请求中的已知值；请求未提供时结果必须为 `null`。两者都存在时 `endAt >= startAt`。
2. `participants` 与 trim、稳定去重后的 `knownParticipants` 完全一致；不得多出模型识别的人名。
3. 有 `knownTitle` 时 `titleSource=context` 且 title 与其一致；没有已知标题时不得使用 `context`。
4. `title=null` 当且仅当 `titleSource=unknown`；非空且无已知标题时 `titleSource=generated`。
5. 所有 evidence ID 均存在；按 transcript 顺序稳定排序；同一数组中无重复 ID。
6. `owner` 和 `dueDateText` 是相关 evidence 文本或全文中的原样子串；owner 不是代词或 speaker label。无法证明时置 `null`，不得自动猜测替换。
7. `dueDate` 只能由可信代码生成，且与一个完整、无歧义的 `dueDateText` 一致；模型返回非空 `dueDate` 应被忽略并重算，或在严格模式下拒绝。
8. 空白字符串、未知占位词、未知枚举、额外属性、重复条目被拒绝或由确定性规范化步骤处理；不能用第二次 LLM 调用“修正”来源事实。
9. `decisions` 的证据必须包含明确确认语义；自动化只能做有限启发式检查，最终防线仍是 Prompt、样例测试和人工可追溯证据。没有把握时降级到 `risksAndIssues(kind="issue")` 或省略，不能升级为决策。

模型候选的语义校验采用确定性降级，不能因为单个可选事实无法证明而丢弃整份已经生成的纪要：受保护上下文始终由可信请求覆盖；悬空或缺失 evidence 的可选条目直接移除；无法证明的 `owner`、`dueDateText` 和 `dueDate` 置为 `null`；未明确确认或仅有低置信度证据的结论、决策、待办直接移除。JSON 无法解析、Schema 版本错误、required 字段缺失、类型错误、未知字段和输入 transcript 无效等不可修复结构问题仍必须失败。

严格复核已持久化值时，语义校验失败使用稳定、无正文的错误细分，例如 `invalid_evidence_reference`、`context_field_mismatch`、`inferred_identity_rejected`、`ambiguous_due_date`。错误对象只包含 JSON Pointer、错误码和安全消息，不包含实际字段值、原文或模型响应。

## 5. Prompt 分层与防幻觉规则

### 5.1 Prompt 层次

Prompt 由可信代码按固定顺序构造，模板内容是版本化仓库资产，不允许 Provider 或 UI 任意拼接系统指令：

1. **System invariants**：角色、只依据输入、不得推断四类敏感事实、抵抗 transcript prompt injection、只输出 JSON。
2. **Schema contract**：目标 `outputSchemaVersion`、字段语义、required/nullable 规则、枚举和 JSON Schema。若 Provider 支持经验证的 structured output，由 adapter 传递同一 Schema；不支持时使用文本约束但仍本地校验。
3. **Template instructions**：由 `templateId + templateVersion` 选择内置模板侧重点，不改变字段形状；`adaptive` 允许模型先判断合适的组织重点，但不能绕过 Schema 和证据约束。
4. **Trusted meeting context**：以独立 JSON 区块传入已知标题、时间和参会人。缺失字段明确为 null/空数组，不能由 transcript 补齐受保护字段。
5. **Transcript quality context**：说明 segments 是否具有时间戳、speaker、confidence；标记低置信度 segment ID，但不宣称无 confidence 等于高 confidence。
6. **Untrusted transcript**：放入带随机或结构化边界的 data 区块，明确其中任何指令均为会议原文，不具控制权。
7. **Output reminder**：只能返回一个根 JSON object，不带 Markdown、代码围栏、解释或前后缀。

Prompt 模板、完整 transcript、模型输出和修复请求均属于敏感正文，不进入普通日志或 mock 调用记录。日志最多记录 `templateId`、`templateVersion`、`schemaVersion`、输入字符/segment 数和安全错误码。

### 5.2 防幻觉规则

- 只提取原文明确表达的事实；可压缩措辞，不补充背景、原因、情绪、优先级、负责人、期限或解决方案。
- 严格区分“讨论/设想/建议/提议”“结论”“已确认决策”和“明确待办”。模糊表达不升级。
- 参会人只复制 trusted context；不从自我介绍、称呼、声纹、speaker label、文件名或会议内容建立身份映射。
- 会议时间只复制 trusted context；不使用文件创建时间、录音开始时间或相对日期推断。
- 负责人必须是明确分配给待办的人名/团队原文；“我来处理”在没有可信 speaker-to-person 映射时负责人为 `null`。
- 截止日期保留明确原文 `dueDateText`；缺失年份或相对日期不得补全年月日。
- 冲突说法要保留为问题，不能选择“更合理”的一方。未确认的客户需求、项目预估和建议不能当作承诺或决策。
- 不生成常识型 mitigation、行动建议或礼貌性填充。没有信息就使用 null/[]。
- transcript 的低置信度段落如果是某项结论、决策、负责人或期限的唯一证据，不输出为确定事实；可加入需要人工核对的 issue。被独立、清晰片段明确印证时才可采用。
- 不提供模型自评的数值置信度；ASR confidence 与纪要事实可信度不是同一概念。

### 5.3 解析与有限修复

- 首选 Provider 已验证的 JSON Schema structured output；能力未知时不能声明支持。
- 文本模式只接受一个根 JSON object。Phase 2 可兼容“仅一个 JSON 代码围栏且围栏外只有空白”的响应；不得用正则从说明文字中捞取局部 JSON。
- JSON parse、Schema 或语义校验必须受 operation deadline 和 cancellation 控制。
- 可配置至多一次**结构修复**，且仅用于 JSON 语法/类型/required 字段问题；必须满足 Provider replay-safety、总超时和重试预算。修复 Prompt 只传目标 Schema、安全的错误路径及必要输入，仍不得记录正文。
- 身份、会议时间、负责人、截止日期、证据不匹配等语义错误不得通过模型修复；模型候选使用确定性覆盖、清空、重算或移除条目，严格复核已持久化值时才拒绝不一致结果。
- 修复失败返回 `schema_validation_failed`，不得保存未校验 JSON，也不得以 `validation.valid=false` 的 envelope 冒充成功。

## 6. Transcript 特殊情况

### 6.1 speaker 与时间戳

- speaker 和时间戳都可缺失。缺失时 Prompt、输出和 UI 保持缺失，不生成 `Speaker 1`、`00:00` 等占位事实。
- speaker label 可帮助区分发言轮次，但只作为不透明 label；不得进入 `participants`，也不能用来解析“我”是谁。
- 时间戳仅用于排序、分块和 UI 定位，不是会议日期/时间。
- `startMs > endMs`、负数、重复 segment ID 等输入不变量由 transcript validator 在生成纪要前拒绝。
- 有 segment 时 evidence ID 应指向最小充分证据集合；不能给每项机械附加全部 segment ID。

### 6.2 空文本与内容不足

- `Transcript.text.trim()` 为空时返回 `empty_transcript`，minutes provider 调用次数必须为 0。
- `Transcript.text` 非空但只有寒暄、噪声标记或无法理解内容时，可生成 Schema-valid 的最小结果：标题/摘要为 null，列表为空；若有可靠证据，可增加一个“内容不足，需人工核对”的 issue。
- `Transcript.text` 与 segments 拼接结果明显矛盾时不得静默选择其一；返回输入校验错误或按 Lead 固化的单一规范来源处理。

### 6.3 低置信度

- 低置信度阈值是应用配置/纪要策略，不写死为 Provider 通用事实；只有 segment 实际含 `confidence` 时才比较。
- 预处理器把低置信度 segment ID 作为质量上下文传入 Prompt，不修改原文、不把 confidence 写成 speaker 信息。
- 低置信度片段可用于发现“需要核对的问题”，但不能单独支持结论、决策、负责人或期限。
- Provider 完全不提供 confidence 时，质量状态是“未知”，不是“全部高置信度”；不得在 UI 或纪要中作高质量保证。

## 7. 长文本分块与合并

### 7.1 何时分块

- 先读取 `MinutesCapabilities.maxInputUnits/inputUnit`。真实能力未知时使用 Lead 配置的保守上限并标注未验证，不能假设某模型上下文大小。
- 预算同时包含 system/template/schema/context、transcript、合并输入和最大输出；必须预留安全余量。
- 输入未超预算时单次生成。超预算时进入确定性 map/reduce；不得把原文简单截断并宣称完整纪要。

### 7.2 分块规则

1. 优先按 `TranscriptSegment` 边界切分，保留原始 segment ID 和顺序；单个超大 segment 再按段落/句子边界切分。
2. 不因分块创造时间戳、speaker 或持久化的伪 segment。纯文本内部切片可使用临时 span ID，但不能输出为 `evidenceSegmentIds`。
3. 块大小由实际 input unit 计算；token 型能力使用与目标模型匹配或保守估计的 tokenizer，不能把字符数假装成 token 数。
4. 相邻块可有配置化的小重叠区以保护跨边界语义。重叠内容必须通过原始 segment ID/内部 span 去重。
5. 每块携带 `chunkIndex/totalChunks`、时间/顺序范围和同一份 trusted context；取消和 overall deadline 在每块与合并阶段均生效。

### 7.3 Map 输出与 Reduce 规则

- Map 阶段使用内部 `ChunkMinutesDraft`，只提取该块的 topics、候选结论、候选决策、候选待办、风险/问题及 evidence；不生成最终参会人和会议时间。
- Reduce 按原文顺序合并，先做 evidence ID/规范化文本的确定性精确去重，再让模型压缩跨块摘要和近似重复项。
- 重叠块中的同一事实不得生成两条；相似但数值、状态、责任或期限冲突的事实不得强行去重，转为 issue 并保留双方 evidence。
- 只有证据明确确认的候选项才能进入最终 `decisions`；“proposal”不能因在多个块重复出现而升级为 decision。
- 负责人和期限不跨块做代词解析。一个块中的“我负责”和另一块中的姓名不能拼接成 owner。
- 最终标题和 summary 可在 Reduce 生成，但不得产生所有 Map 结果和 evidence 中都不存在的新事实。
- Reduce 后必须重新执行完整 JSON Schema 与语义校验。分块中间结果不持久化为最终纪要，也不进入普通日志。

## 8. 内置模板契约

内置模板共享 `MeetingMinutes` v1.0.0。模板 ID/首版版本如下；实际注册表由 Rust 模块实现并测试。

| `templateId` | `templateVersion` | 内容侧重点 | 禁止升级的内容 |
| --- | --- | --- | --- |
| `standard_meeting` | `1.0.0` | 平衡摘要、议题、结论、决策、待办和风险；按讨论顺序组织 topics | 一般讨论不能自动变成决策/待办 |
| `project_weekly` | `1.0.0` | topics 优先按项目/工作流归类；summary 强调已报告进展；risksAndIssues 强调阻塞、依赖与偏差；actionItems 只收明确后续动作 | 进度描述不是结论；目标日期不是待办截止日期；模型不能推算完成率 |
| `customer_communication` | `1.0.0` | topics 聚焦客户诉求、澄清和约束；decisions 仅收双方明确确认项；actionItems 收明确对外/客户动作；issues 保留异议和待确认问题 | 客户建议不等于我方承诺；销售意向不等于决策；不能推断客户情绪、身份或合同义务 |
| `course_summary` | `1.0.0` | 提炼课程主题、核心概念、知识结构、案例和明确学习任务 | 讲师观点不自动升级为客观事实；不得补造课程目标或作业 |
| `research_project` | `1.0.0` | 聚焦研究问题、方法、证据、阶段结论、局限和后续研究动作 | 假设不等于结论；相关性不等于因果；不得补造实验结果 |
| `academic_lecture` | `1.0.0` | 组织讲座主题、理论框架、论证链、案例、争议和启发 | 演讲者推测不等于学界共识；不得补造引用和数据来源 |
| `profile_interview` | `1.0.0` | 围绕人物经历、关键事件、观点、选择与原话证据组织 | 不推断人格、动机和未明确身份；编辑性归纳需保留证据 |
| `in_depth_interview` | `1.0.0` | 围绕访谈问题、回答脉络、关键洞察、矛盾点和待核实事项组织 | 不将提问者假设当作受访者观点；不得消除有意义的矛盾 |
| `business_plan` | `1.0.0` | 提炼机会、客户、价值主张、方案、商业模式、资源、里程碑和风险 | 设想不等于已验证事实；收入预测不等于承诺；不得补造市场数据 |
| `article_outline` | `1.0.0` | 将内容整理为中心论点、章节脉络、论据、案例和待补材料 | 不补写转写中没有的论据；缺口应作为风险或待办呈现 |
| `adaptive` | `1.0.0` | 根据转写内容判断最合适的上述组织重点，并在统一 Schema 中输出 | 不回显或伪造模板身份；不改变字段；不以“自适应”为由放宽证据规则 |

模板只能调整字段选择优先级、组织方式和措辞，不得：

- 改变 JSON 字段、nullable 规则或证据规则；
- 放宽身份、时间、负责人、期限来源限制；
- 覆盖 System invariants；
- 要求输出原 Schema 不允许的自由文本/Markdown；
- 由 UI 提供任意 system Prompt。自定义模板编辑器不属于 MVP。

## 9. Phase 2 样例输入/输出计划

fixture 必须是人工编写、无真实会议内容、无真实姓名/内部地址/密钥的确定性样例。不得从仓库根目录测试录音转写后制作 fixture。建议布局：

```text
shared/fixtures/minutes/v1/
├─ valid/
│  ├─ standard-complete.request.json
│  ├─ standard-complete.minutes.json
│  ├─ no-context-no-segments.request.json
│  ├─ no-context-no-segments.minutes.json
│  ├─ project-weekly-low-confidence.request.json
│  ├─ project-weekly-low-confidence.minutes.json
│  ├─ customer-proposal-vs-commitment.request.json
│  ├─ customer-proposal-vs-commitment.minutes.json
│  ├─ explicit-owner-relative-date.request.json
│  └─ explicit-owner-relative-date.minutes.json
├─ long/
│  ├─ cross-chunk-overlap.request.json
│  ├─ cross-chunk-overlap.minutes.json
│  ├─ conflicting-statements.request.json
│  └─ conflicting-statements.minutes.json
└─ invalid/
   ├─ missing-required-field.json
   ├─ additional-property.json
   ├─ inferred-participant-from-speaker.json
   ├─ meeting-time-without-context.json
   ├─ dangling-evidence-id.json
   ├─ inferred-owner-from-pronoun.json
   ├─ inferred-absolute-due-date.json
   ├─ decision-from-proposal.json
   ├─ whitespace-placeholder.json
   └─ schema-version-mismatch.json
```

样例至少证明：

- 完整 trusted context 能原样进入受保护字段；无 context 时 meeting time 为 null、participants 为 []。
- 匿名 speaker、自我介绍和“我负责”不能产生参会人/owner。
- “由测试团队负责，明天下午前完成”可保留 owner 和 `dueDateText`，但 `dueDate=null`。
- 提议、客户诉求、已确认决策分别进入不同语义位置。
- 低置信度唯一证据不会形成确定决策/负责人/期限。
- 分块重叠去重、跨块冲突保留、顺序稳定。
- 不提供 segments 时 evidence 数组为空，但不虚构 ID/时间戳。

## 10. Phase 2 校验与解析测试矩阵

| 层 | 必测场景 | 预期 |
| --- | --- | --- |
| JSON parser | 合法根对象、前后空白、单一可选 JSON fence | 成功且结果一致 |
| JSON parser | 前缀解释、两个 JSON、截断 JSON、超深嵌套、超响应上限 | 拒绝，不做局部捞取，不泄露 body |
| Schema | 标准/周会/客户 valid fixtures | 全部通过同一个 `1.0.0` Schema |
| Schema | 缺 required、额外字段、错误 nullable、非法 enum、空白字符串、数组/长度超限 | 精确失败，返回安全 JSON Pointer |
| Version | request/payload/envelope/validation 版本一致与不一致 | 一致通过；不一致为 `schema_version_mismatch` |
| Context | 已知 title/time/participants 原样复制 | 通过 |
| Context | 无 context 却生成时间/参会人，或增加 context 外姓名 | `context_field_mismatch` / `inferred_identity_rejected` |
| Evidence | 有效、重复、乱序、悬空 ID、无 segments | 有效通过；重复稳定去重；乱序归一；悬空拒绝；无 segments 接受 [] |
| Owner | “由测试团队负责”、代词、speaker label、跨块拼接 | 仅明确原文团队通过，其余为 null/拒绝 |
| Due date | 完整公历日期、缺年份、相对日期、模型私填 date | 完整日期由可信代码规范化；其余 `dueDate=null` 并保留原文 text |
| Semantics | proposal/decision/conclusion/action 的正反样例 | 不允许 proposal 升级；边界结果与 golden fixture 一致 |
| Confidence | 唯一低置信证据、被高质量片段印证、无 confidence | 分别省略/标为 issue、允许采用、保持质量未知 |
| Empty | 空串、全空白、仅噪声、极短有效文本 | 空白不调用 provider；其余得到诚实的 null/[] 或有限 issue |
| Chunking | 边界、重叠、单 segment 超长、冲突、取消、总超时 | 不截断/不重复/不消解冲突；可取消并遵守 deadline |
| Repair | 一次结构修复成功/失败、语义错误、取消、replay-unsafe | 仅允许预算内结构修复；语义错误不交给模型修复 |
| Determinism | 同 fixture 多次 parser/normalizer/merge | 输出字段顺序、数组稳定顺序和错误码一致 |
| Security | sentinel transcript/key 出现在 provider 错误、校验错误和日志扫描 | 日志、错误、snapshot、测试报告均不出现原值 |
| Envelope | 未校验结果、Schema-valid、语义-invalid、late success after cancel | 仅双重校验通过且未取消者生成成功 envelope |

测试应分为：纯函数单元测试（parse/normalize/semantic rules）、Schema fixture tests、Prompt snapshot 的结构测试（snapshot 不含完整敏感 transcript）、长文本合并测试，以及通过 Agent 4 `MinutesProvider` mock 的集成测试。Prompt snapshot 使用占位标记或短小非敏感 fixture，不能复制本地真实录音的内容。

## 11. 跨 Agent 接口约束

- **Agent 4 / Provider**：只接收 `MinutesRequest` 并返回候选模型 JSON；调用 Agent 5 唯一 validator 后才能构造成功 `MeetingMinutesEnvelope`。Provider 不复制 Schema，不解释 template，不把 raw response 写日志。
- **Lead / Task Orchestrator**：负责把 trusted `meetingContext`、template/schema version、deadline 与 cancellation 传入；确保 payload/envelope 版本一致；只有校验通过的纪要进入 SQLite 和 `completed`。
- **Agent 2 / UI**：按 null/[] 渲染空状态，不把 speaker label 显示成真实参会人，不把 `dueDateText` 相对日期伪装为绝对日期；`title=null` 时显示本地 UI 占位，不回写为业务 title。
- **持久化/导出**：保存原版本 JSON 和模板元数据；完整 transcript 单独存储。Markdown 按稳定章节顺序渲染 null/[]，不得通过渲染补齐未知负责人或日期。
- **Agent 6 / QA**：覆盖受保护字段来源、低置信度、prompt injection、长文本遗漏/重复、结构修复预算和 sentinel 泄露扫描。

## 12. 未决项与阶段门

以下事项在 Phase 0 保持未决，不阻塞 mock-first 骨架，但阻塞宣称真实 Provider 已完成：

- 真实 LLM 的 structured output / JSON Schema Draft 支持度、最大输入/output 和 token 计算方式；
- 真实 Provider 是否会返回代码围栏、拒答、安全过滤或截断 JSON，以及对应可安全修复策略；
- 低置信度阈值和 Schema 字符/数组上限的最终配置值；
- 模板中文措辞和 Markdown 章节文案的产品验收；
- 没有 segments 时是否需要引入独立、非伪造的文本 span evidence 类型；v1.0.0 先允许空 evidence 数组。

Phase 2 只有在 Schema 文件、Rust parser/validator、模板注册表、人工 fixtures 和上述测试真实执行通过后，才能声明纪要模块完成。Phase 4 必须用脱敏、最小非敏感输入验证真实模型结构；不可访问时明确标记 `BLOCKED` 并继续使用 mock，不得根据相似 API 推测成功。
