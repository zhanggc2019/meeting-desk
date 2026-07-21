# 云端 API 与 Provider 契约

> 状态：Phase 2 实现基线
> 日期：2026-07-17
> 适用范围：`TranscriptionProvider`、`MinutesProvider`、OpenAI-compatible 传输层、mock provider 及 Phase 4 真实接口验证
> 重要声明：截至 Phase 2，项目仍未调用任何真实 ASR/LLM API，也未验证任何供应商的路径、请求字段或响应字段。当前 HTTP adapter 只能使用调用方提供的显式字段映射；出现的 HTTP 结构均不代表某个真实供应商协议。

## 1. 目标与非目标

本契约使任务编排、UI、数据库和测试不依赖具体云厂商。核心约束如下：

- `TranscriptionProvider` 只负责把 ImportService 生成的受控离线音频引用转换为中立 `Transcript`。
- `MinutesProvider` 只负责提交由纪要模块构造的敏感 Prompt/Schema，并返回不受信任的 `MinutesCandidate`；纪要模块使用唯一 validator 校验后才能构造 `MeetingMinutesEnvelope`。
- endpoint、model、timeout、retry、并发与凭据引用均配置化；UI 和数据库不持有密钥值。
- OpenAI-compatible 仅表示“可由一个兼容适配器族承载”，不表示路径、multipart 字段、鉴权、异步轮询、JSON 字段或 structured output 完全相同。
- 所有网络操作必须可取消、有明确超时、有错误分类，并在重试前判断请求能否安全重放。
- 普通日志、UI 错误、IPC 事件和测试报告不得包含 API Key、Authorization、音频内容、完整转写、完整纪要、完整 prompt 或 HTTP body。

以下内容不在 Phase 0 中定义：

- 任何真实供应商的私有请求/响应字段；
- 未经实测的大小、时长、格式、速率或幂等保证；
- Agent 5 负责的会议纪要 JSON Schema 具体字段细节；
- 任何音频设备输入、实时音频或流式 ASR 协议。MVP 只处理用户显式选择的单个或批量离线音频文件。

## 2. 分层与依赖方向

```text
User-selected offline file(s)
  -> ImportService (read-only validation + staging metadata)
  -> AudioArtifactRef
  -> Task Orchestrator
  │ stable internal DTO + CancellationToken
  ├── TranscriptionProvider ─┐
  └── MinutesProvider ───────┤
                             v
                    Provider Adapter / Codec
                    - capabilities
                    - request mapping
                    - response mapping
                    - replay-safety policy
                             │ HttpRequestSpec
                             v
                       Shared HTTP Transport
                    - secret injection
                    - timeout / cancel
                    - concurrency / cooldown
                    - safe metadata only
                             │ HTTPS
                             v
                       Untrusted Provider
```

依赖只能向下：

1. UI、任务编排和持久化只认识内部 DTO 与 `ProviderError`，不得解析供应商 JSON。
2. adapter/codec 是唯一允许知道供应商字段映射的模块。
3. 通用 HTTP 层只执行 adapter 生成的 `HttpRequestSpec`，不得理解 transcript 或 minutes 业务字段。
4. adapter 返回内部 DTO 前必须完成响应大小限制、解码、字段归一化和基础不变量检查。
5. 会议纪要最终的业务 Schema 校验由 minutes validator 完成；通过前不得进入 `completed`。

推荐 Phase 1 物理边界：

```text
src-tauri/src/providers/
├─ mod.rs
├─ contract.rs          # traits、内部 provider DTO、能力声明
├─ error.rs             # ProviderError 与安全映射
├─ transport.rs         # 统一 HTTP transport
├─ retry.rs             # 重试/Retry-After/退避
├─ rate_limit.rs        # 并发与共享 cooldown
├─ openai_compatible/   # 经验证后增加 codec；Phase 0 不预设字段
└─ mock/                # 确定性 mock provider
```

最终模块名可由 Lead 在 Phase 1 固化，但不得破坏上述分层。

## 3. 稳定内部 DTO

以下为语言无关的规范性形状。Phase 1 可分别生成 Rust 类型和 TypeScript IPC 类型；类型名称调整须由 Lead 统一，字段语义不可由具体 adapter 改写。

### 3.1 通用标识与导入音频引用

```text
ProviderId = opaque string
OperationId = UUID generated locally for one logical provider operation
AttemptId = UUID generated locally for one network attempt
TaskId = opaque persistent task id

AudioArtifactRef {
  id: opaque imported-audio id
  importBatchId?: opaque batch id
  sourceKind: "UserSelectedFile"
  stagingMetadata: {
    mimeType: string
    byteLength: non-negative integer
    durationMs?: non-negative integer
    sha256?: lowercase hex     # 只用于本地完整性/去重，不进入普通日志
    validatedAt: UTC timestamp
  }
}
```

- `AudioArtifactRef` 只能由 ImportService 在用户通过文件选择器显式选择文件后创建。Provider、UI 和 task orchestrator 不得自行接受任意路径字符串并绕过导入校验。
- ImportService 以只读方式检查文件存在、可读、非空、容器/MIME、可解析元数据与配置化大小限制；不能只相信扩展名，也不得修改用户源文件。
- `stagingMetadata` 是受控引用的元数据，不表示已创建第二份音频副本。原始绝对路径、文件名和只读 handle 只保留在 Rust Core 的受信任导入注册表中，不进入 Provider DTO、IPC 或普通日志。
- adapter 只能通过 ImportService/受信任 artifact resolver 获取只读 stream/handle，不得把整段音频载入 WebView 或日志。
- `stagingMetadata.byteLength == 0` 必须在调用 provider 前返回 `empty_audio`。
- 在开始上传前，应再次确认源文件未被替换、截断或修改；不一致返回 `source_file_changed`，不得上传与用户选择时不同的内容。
- 取消、失败、重试或删除任务均不得删除、移动、重命名或改写用户源文件。若未来引入应用自有临时副本，其生命周期必须与用户源文件严格分离，并由显式清理策略管理。
- 批量选择时，每个文件生成独立 `AudioArtifactRef`；`importBatchId` 只用于分组和批量操作，不使各文件共享成功/失败终态。

### 3.2 转写 DTO

```text
TranscriptionOptions {
  languageHint?: BCP-47-like string
  enableTimestamps: boolean
  enableSpeakerLabels: boolean
  enableConfidence: boolean
  domainHintId?: opaque template/config id
}

TranscriptSegment {
  id: stable segment id within transcript
  startMs?: non-negative integer
  endMs?: non-negative integer
  speakerLabel?: opaque provider-neutral label
  text: string
  confidence?: number in [0, 1]
}

Transcript {
  schemaVersion: "1"
  text: string
  language?: string
  durationMs?: non-negative integer
  segments: TranscriptSegment[]
  providerMetadata: ProviderResultMetadata
}

ProviderResultMetadata {
  providerId: ProviderId
  adapterId: string
  adapterVersion: string
  model?: string
  remoteRequestId?: string
  startedAt: UTC timestamp
  completedAt: UTC timestamp
}
```

归一化规则：

- `text` 是完整转写，不能为空白；空白结果映射为 `empty_transcript`，不会调用 `MinutesProvider`。
- 不提供 segment 的 provider 返回 `segments: []`，不得由 UI 或 adapter 虚构时间戳、speaker 或 confidence。
- 如果同时存在 `startMs` 与 `endMs`，必须满足 `startMs <= endMs`；confidence 必须在 `[0, 1]`。
- `speakerLabel` 只是匿名标签，不能映射为真实参会人姓名。
- `providerMetadata` 不包含原始响应、计费 payload、prompt、转写内容或密钥。
- 远端 request id 只有在已确认该响应头/字段不含凭据或用户正文时才可保留；否则省略。

### 3.3 会议纪要调用 DTO

```text
MinutesRequest {
  transcript: Transcript
  templateId: string
  templateVersion: string
  outputSchemaVersion: string
  meetingContext?: {
    knownTitle?: string
    knownStartAt?: UTC timestamp
    knownEndAt?: UTC timestamp
    knownParticipants?: string[]
  }
}

MeetingMinutesEnvelope {
  schemaVersion: string
  minutes: MeetingMinutes       # 具体结构由 shared JSON Schema 定义
  validation: {
    valid: true
    schemaVersion: string
  }
  providerMetadata: ProviderResultMetadata
}

MinutesCandidate {
  schemaVersion: string
  value: untrusted JSON value
  providerMetadata: ProviderResultMetadata
}
```

- `MeetingMinutes` 的唯一规范来源是 Agent 5 维护的版本化 JSON Schema，provider 模块不得复制一份分叉 Schema。
- `MinutesCandidate` 不表示业务成功，不能直接持久化或展示；它必须交给 Agent 5 维护的唯一 parser/Schema/semantic validator。
- `MeetingMinutesEnvelope` 只能由纪要 validator 在候选通过目标 Schema 和语义校验后构造；校验失败返回 `schema_validation_failed`，不能以 `valid: false` 的 envelope 冒充成功。
- `Transcript` 正文与 prompt 会作为敏感 request body 发送给用户配置的 provider，但不会出现在 transport 日志或 provider metadata 中。
- 用户未提供的参会人、时间或负责人信息保持缺失/空值，不得根据 speaker label 推测。

## 4. Provider 行为接口

下列 TypeScript 风格 IDL 用于表达行为，不是 Phase 0 的可编译实现。每个方法都必须在 Rust trait 中具有等价语义。

```ts
interface TranscriptionProvider {
  /** 返回当前 adapter 经验证的能力；不得把未知限制表示为已支持。 */
  capabilities(): Promise<TranscriptionCapabilities>;

  /** 校验 ImportService 生成的离线音频引用和选项，并在成功时返回完整、归一化的转写。 */
  transcribe(
    context: ProviderOperationContext,
    artifact: AudioArtifactRef,
    options: TranscriptionOptions,
  ): Promise<Transcript>;
}

interface MinutesProvider {
  /** 返回 structured output、Schema 和输入上限等已验证能力。 */
  capabilities(): Promise<MinutesCapabilities>;

  /** 生成不受信任的 JSON 候选；调用方必须使用唯一纪要 validator 校验。 */
  generateCandidate(
    context: ProviderOperationContext,
    request: MinutesRequest,
  ): Promise<MinutesCandidate>;
}

interface ProviderOperationContext {
  taskId: string;
  operationId: string;
  cancellationToken: CancellationToken;
  deadlineAt: string;
  attemptObserver: AttemptObserver;
}
```

统一行为：

- 一个方法调用代表一个逻辑 operation，可由内部多个 HTTP attempt 或异步提交/轮询构成。
- `deadlineAt` 是 operation 总截止时间；单次 request timeout 不能突破它。
- `cancellationToken` 必须传播到排队、上传、等待退避、异步轮询、响应读取与解析边界。
- provider 方法不直接持久化会议、不更新 UI、不把正文写入日志；由 task orchestrator 管理任务状态。
- 成功只返回内部 DTO；失败只返回 `ProviderError`；不得把 `reqwest::Error`、原始 body 或供应商对象穿过 IPC。
- 未知的远端进度只报告阶段，不伪造百分比。

### 4.1 能力声明

```text
TranscriptionCapabilities {
  evidence: Verified | Mock | Unverified
  acceptedMimeTypes: string[]
  maxAudioBytes?: integer
  maxDurationMs?: integer
  supportsAsyncJobs: boolean
  supportsTimestamps: boolean
  supportsSpeakerLabels: boolean
  supportsConfidence: boolean
  supportsRemoteCancel: boolean
  replaySafety: ReplaySafety
}

MinutesCapabilities {
  evidence: Verified | Mock | Unverified
  supportsJsonSchema: boolean
  supportedSchemaVersions: string[]
  maxInputCharacters?: integer
  supportsAsyncJobs: boolean
  supportsRemoteCancel: boolean
  replaySafety: ReplaySafety
}

ReplaySafety =
  | AlwaysSafe
  | SafeWithVerifiedIdempotencyKey
  | BeforeRequestBodySentOnly
  | NeverAutomaticallyReplay
```

规则：

- `Verified` 只用于 Phase 4 已用真实接口或权威供应商契约确认的能力；必须关联验证记录。
- `Mock` 只描述测试实现，不得用于宣称真实 API 兼容。
- 未知数值限制用缺失值，不得使用任意“大数”代替。
- 如果请求要求时间戳等非必需能力但 provider 不支持，adapter 应在 preflight 返回 `unsupported_option`，或由产品明确降级并通知用户；不得静默伪造。
- `ReplaySafety` 由 adapter 基于真实协议证据声明，通用重试层不得自行猜测。

## 5. OpenAI-compatible 可配置请求层

### 5.1 “兼容”的严格定义

本项目把 OpenAI-compatible 设计为 adapter family，而不是固定 URL 或固定 JSON：

```text
Internal DTO
   -> adapter-selected request codec
   -> HttpRequestSpec
   -> shared transport
   -> RawHttpResponse (size-limited, sensitive)
   -> adapter-selected response codec
   -> Internal DTO
```

`HttpRequestSpec` 至少包含 method、经校验的 endpoint、非敏感 header、受保护 credential 注入指令、streaming body、单次 timeout 和 response size limit。它是 Rust 内部敏感对象，不经 IPC、不序列化到普通日志。

禁止以下做法：

- 在 UI、task orchestrator 或数据库中判断供应商 JSON 字段；
- 把一个供应商的路径、multipart 字段名、异步 job 字段写成所有 OpenAI-compatible provider 的默认真相；
- 允许用户用任意脚本/模板读取本地文件或注入任意敏感 header；
- 未经 Phase 4 验证就把某 codec 标记为 production-ready。

### 5.2 配置模型

逻辑配置如下，具体 Rust/SQLite 字段由 Lead 在 Phase 1 统一：

```text
ProviderProfile {
  id: ProviderId
  kind: Mock | OpenAiCompatible | ProviderSpecificAdapter
  adapterId: string
  adapterVersion: string
  endpoint: absolute URL
  model: string
  credentialRef?: opaque Windows Credential Manager reference

  connectTimeoutMs: integer
  requestTimeoutMs: integer
  overallTimeoutMs: integer
  maxRetries: integer
  retryBaseDelayMs: integer
  retryMaxDelayMs: integer
  maxRetryAfterMs: integer

  maxConcurrent: positive integer
  minRequestIntervalMs: non-negative integer
  maxResponseBytes: positive integer

  transcription?: {
    maxAudioBytes?: integer
    pollingIntervalMs?: integer
  }
  minutes?: {
    temperature?: number
    templateId: string
  }
}
```

配置约束：

- endpoint 必须是绝对 URL。生产模式只允许 HTTPS；开发模式仅可显式放行 loopback 地址的 HTTP mock server。
- endpoint 禁止包含 userinfo；query 参数默认禁止，确有供应商要求时只能由经审查 adapter 生成，且日志中删除整个 query。
- `adapterId + adapterVersion` 决定字段映射，不由 `providerId` 分支硬编码。
- model 不能为空，但是否有效只能由真实调用确认；配置测试失败不能回显远端 body。
- `maxRetries` 指失败后的额外尝试次数，总 attempt 上限为 `1 + maxRetries`。
- timeout、并发和 response 大小必须设置全局安全上下限，避免 `0` 被解释为无限等待。
- `credentialRef` 可以持久化，credential value 不可进入 SQLite、前端 store、配置导出或调试 dump。
- 开发环境变量只用于注入 credential value，`.env.example` 只列空占位，不含测试 Key。

### 5.3 鉴权与 header

- SecretStore 在真正发起请求前按 adapter 的 allowlisted auth strategy 注入凭据。
- Phase 0 不假设所有 provider 使用 Bearer；Bearer、API-key header 或其他策略必须各自由已验证 adapter 声明。
- 不允许用户从 UI 配置任意 secret header 名和值。若新增鉴权策略，需代码审查、redaction 测试和 Phase 4 记录。
- `Authorization`、`Proxy-Authorization`、`Cookie`、`Set-Cookie`、包含 `api-key`/`token`/`secret`/`credential` 的 header 始终按敏感字段处理。
- transport 不启用会打印 request/response header 或 body 的 HTTP trace。

## 6. 生命周期、异步任务与取消

### 6.1 离线文件任务阶段

每个导入文件是独立任务，Provider 相关阶段统一为：

```text
queued -> uploading -> transcribing -> summarizing -> validating -> saving -> completed
```

- `uploading` 表示正在排队获取上传许可或正在发送离线文件字节；若协议将上传和转写合并为同一 HTTP 请求，本地只在 request body 确认发送结束后切换为 `transcribing`。
- `transcribing` 表示等待同步转写结果，或等待异步转写 job 的终态；provider 不提供进度时只展示阶段，不伪造百分比。
- `summarizing` 表示向 `MinutesProvider` 提交 transcript 并等待结构化结果；开始该阶段前必须确认 transcript 非空。
- 一个文件失败或取消不改变同批其他文件的终态。批量操作只是独立任务的分组控制，不是共享事务。

各阶段取消语义：

| 取消位置 | 必须行为 | 远端结果语义 |
| --- | --- | --- |
| 排队/等待限流 | 从队列移除，不打开用户文件，不发请求 | `NotSent` |
| 上传前 | 释放许可与只读 handle，不改动用户源文件 | `NotSent` |
| 上传中 | 中止 body stream/HTTP request，释放只读 handle；不得删除用户源文件 | 已发送部分字节时为 `Unknown` |
| 转写中（同步） | 中止本地等待并丢弃 late response | provider 未确认取消时为 `Unknown` |
| 转写中（异步） | 有已验证远程 cancel 时 best-effort 调用；无能力时停止 poll | `remote-confirmed` 或 `remote_state_unknown` |
| 总结中 | 中止 request/响应读取/Schema 解析，丢弃 late response | body 已发送且无远程取消时为 `Unknown` |
| 校验/保存中 | 停止尚未提交的后续步骤；不得把部分结果标为完成 | 远端调用已结束，本地任务为 `cancelled` 或安全回滚结果 |

单文件取消只触发该 task 的 token；“取消整批”触发 batch parent token，并传播到仍在排队及所有活动子任务。传播后的每个子任务仍按自己的发送阶段计算 `outcome`，不能把整批统一伪报为远端已取消。

### 6.2 同步 provider

`transcribe`/`generate` 在一次 request-response 中完成。取消或 deadline 到达时：

1. 立即停止排队、上传、转写/总结等待或响应读取；
2. 释放 response/body/file handle；
3. 返回 `cancelled` 或对应 timeout；
4. 忽略随后到达的 late response；
5. 不产生成功 DTO，不进入任务 `completed`。

释放 file handle 只结束应用的只读访问，不删除、移动或修改用户选择的离线文件。

### 6.3 异步 provider

若真实 adapter 经验证支持 submit/poll：

- adapter 把 submit 与 poll 封装在一次 provider operation 内部；UI 不接触 remote job JSON。
- remote job id 作为敏感度受控的 adapter 状态，只能在确认不含正文/凭据后以安全字段持久化，用于重启恢复。
- poll 使用同一 operation deadline、共享取消 token 和速率限制器。
- provider 有经验证的远程 cancel API 时，用户取消先做 best-effort remote cancel，再停止轮询。
- provider 没有远程 cancel 或 cancel 失败时，本地仍进入 `cancelled`，并记录安全状态 `remote_state_unknown`；必须提示“远端任务可能继续执行/计费”，不得伪称远端已取消。
- 重启恢复前必须确认 remote job id、adapter version 与 credential reference 一致，否则标记 `interrupted`，等待用户重试。

### 6.4 竞态规则

- 每个 operation/attempt 使用唯一 id；只有当前活动 attempt 可以提交结果。
- cancel 胜出后，任何 late success 都丢弃，不写入 transcript/minutes。
- retry 启动新 `AttemptId`，但保留同一 `OperationId` 和已验证的 provider idempotency key。
- operation 完成或取消后再次调用 cancel 返回当前终态，不能触发第二次远程操作。
- 等待 retry backoff 或 rate-limit cooldown 时也必须可取消。
- 取消必须尽快释放对应 ASR/LLM semaphore permit，使同批其他离线文件可以继续推进。

## 7. 超时模型

| 超时 | 范围 | 到期错误 | 说明 |
| --- | --- | --- | --- |
| `connectTimeoutMs` | DNS/TCP/TLS 建连 | `connect_timeout` | 不含服务端处理时间 |
| `requestTimeoutMs` | 单个 HTTP attempt | `request_timeout` | 包含上传、服务端等待与响应读取；异步 poll 每次单独计算 |
| `overallTimeoutMs` | 整个 provider operation | `operation_timeout` | 包括排队、退避、提交、轮询、解码；最高优先级 |

规则：

- 实际 attempt deadline 取 `request deadline` 与 `operation deadline` 中更早者。
- 任何 timeout 都要先判断 `ReplaySafety`，不是所有 timeout 都能自动重试。
- request body 已部分或全部发出后发生 timeout，远端是否接收成功通常未知；adapter 未证明幂等时返回 `outcome_unknown`，默认不自动重放。
- timeout 不允许通过把值设为零而关闭。
- 解析/Schema 校验也受 overall deadline 约束，避免超大恶意响应长期占用资源。

## 8. 错误契约与 HTTP 分类

```text
ProviderError {
  code: stable machine-readable code
  category: Configuration | Input | Authentication | Permission |
            RateLimit | Network | Timeout | Provider | Response |
            Cancellation | LocalResource
  retryable: boolean
  replaySafe: boolean
  safeMessage: localized-safe message key or text
  httpStatus?: integer
  retryAfterMs?: integer
  outcome: NotSent | Rejected | Failed | Unknown
  remoteRequestId?: string
}
```

`safeMessage` 是 IPC/UI 可见信息；底层 exception chain、URL query、headers、body、prompt 和正文不能包含在其中。

| 条件 | 稳定错误码 | 默认自动重试 | 说明 |
| --- | --- | --- | --- |
| provider/credential 未配置 | `provider_not_configured` | 否 | 引导用户设置 |
| endpoint 非法/生产 HTTP | `invalid_provider_endpoint` | 否 | 请求发出前失败 |
| 音频为空/损坏/不支持 | `empty_audio` / `corrupt_audio` / `unsupported_audio` | 否 | ImportService preflight 失败 |
| 导入后源文件被修改/替换 | `source_file_changed` | 否 | 上传前只读复核失败；要求用户重新选择文件 |
| 不支持所请求能力 | `unsupported_option` | 否 | 不静默伪造 |
| HTTP 400/404/405/409/422 等结构性 4xx | `provider_request_rejected` | 否 | adapter 可细分，但不得盲目重试 |
| HTTP 401 | `http_401` | 否 | 凭据无效或缺失；不回显 body |
| HTTP 403 | `http_403` | 否 | 权限/策略问题；不回显 body |
| HTTP 408 | `http_408` | 条件性 | 仅在 replay-safe 时重试 |
| HTTP 413 | `http_413` | 否 | 标记大小限制；不能自动切片，除非 adapter 已定义并验证 |
| HTTP 429 | `http_429` | 条件性 | 尊重有上限的 `Retry-After`，且必须 replay-safe |
| HTTP 500/502/503/504 | `http_5xx` | 条件性 | 仅 replay-safe 且未取消/未超总时限 |
| DNS/连接拒绝/断网 | `network_unavailable` | 条件性 | 未发送 body 时通常可重试；以 transport 证据为准 |
| 连接超时 | `connect_timeout` | 条件性 | `outcome=NotSent` 时可按策略重试 |
| request/operation timeout | `request_timeout` / `operation_timeout` | 条件性/否 | operation timeout 永不再重试 |
| 响应超限 | `response_too_large` | 否 | 立即停止读取 |
| 非法 JSON/字段缺失 | `invalid_provider_response` | 默认否 | 可由 adapter 定义至多一次且需 replay-safe；保留安全结构诊断 |
| transcript 为空 | `empty_transcript` | 默认否 | 不调用 minutes provider |
| minutes Schema 不通过 | `schema_validation_failed` | 默认否 | repair/retry 策略由 minutes 模块显式控制 |
| 用户取消 | `cancelled` | 否 | 终态，无后续成功写入 |

未知 HTTP status 不能直接标为 retryable。adapter 没有证据时采用失败关闭：`retryable=false`、`outcome=Unknown`。

## 9. 重试、退避与速率限制

### 9.1 自动重试门槛

只有同时满足以下条件才可自动重试：

1. 错误类别被策略列为临时错误；
2. 当前 adapter 的 `ReplaySafety` 允许当前发送阶段重放；
3. attempt 未超过 `1 + maxRetries`；
4. operation deadline 尚有足够时间；
5. cancellation 未触发；
6. 共享 provider cooldown 已满足；
7. 对未知结果的请求，有真实 provider 幂等保证或已验证 idempotency key。

对于 `outcome=Unknown` 且没有已验证幂等机制的请求，自动重试必须停止并让用户决定，避免重复计费或重复远端 job。

### 9.2 退避

- 默认采用 capped exponential backoff + full jitter；确切默认值由 Phase 1 配置基线决定并测试。
- 429/503 若有合法 `Retry-After`，优先使用它；同时受 `maxRetryAfterMs` 和 operation deadline 限制。
- `Retry-After` 可以是 delta-seconds 或 HTTP-date；解析失败时回退到本地退避，不直接使用原始 header 文本。
- 服务端建议等待超过 operation deadline 时直接返回 `operation_timeout`，不无限挂起。
- retry wait 状态只公开下一次尝试时间、attempt 和安全错误码，不公开响应 body。

### 9.3 速率限制与批量公平性

- 速率限制 key 至少由 `providerId + credentialRef + operationKind` 组成，避免一个配置拖垮全部 provider。
- 每个 key 使用配置化并发 semaphore；MVP 默认应保守，不假定供应商额度。
- ASR 上传/转写与 LLM 总结分别排队、分别持有 semaphore，已完成转写的文件不能绕过总结队列无限抢占其他文件。
- `minRequestIntervalMs` 用于已知最低间隔；未知 QPS 不编造数值。
- 收到 429 后设置该 key 的共享 cooldown，使同批其他任务也尊重等待，而不是各自立即撞限流。
- 每个阶段队列必须 FIFO 或提供等价公平性；单个超大文件、连续 retry 或较早批次不能永久饿死同批/后续批次中的其他文件。
- retry 完成退避后重新进入对应阶段的公平队列，不能始终插到队首形成 retry storm。
- 一个文件失败或取消只释放自身 permit、状态和 retry 计划，不暂停同批其他文件。
- 等待 semaphore/cooldown 可按单文件或整批取消，且计入该文件的 overall timeout。
- 某一 provider key 的 429 cooldown 只影响使用该 key 的对应 operation kind；不得无理由阻塞其他 provider、其他 credential 或纯本地步骤。
- provider 未提供配额 header 时不伪造剩余额度或进度。

## 10. 幂等、重复提交与单飞边界

本地去重与远端幂等是两个独立问题。

### 10.1 本地任务去重

- task orchestrator 对同一 `AudioArtifactRef` 的同一处理配置建立“单活动任务”约束。
- 建议去重键由 `AudioArtifactRef.id + operation kind + provider profile version + model + template/schema version` 组成；不能只用文件名或批次 id。
- staging metadata 中的 SHA-256 可用于完整性和用户确认后的跨引用去重，但不写普通日志，也不能仅凭哈希自动复用不同隐私上下文中的结果。
- 重复点击返回现有活动 task id，不创建第二条并行云请求。
- 已完成任务的重新生成是新的用户动作，必须有新 task/operation 记录，不能静默覆盖原结果。

### 10.2 远端幂等

- 只有在供应商文档或真实验证明确支持时，adapter 才可发送 idempotency key。
- 同一逻辑 operation 的所有 retry 使用同一 provider idempotency key；新用户任务使用新 key。
- header/字段名、作用域、保存期限与冲突语义属于 provider mapping，不能写进通用层假设。
- 如果 provider 不支持幂等，发送 body 后出现网络断开/timeout 视为 `outcome=Unknown`，默认禁止自动重试。
- 异步 submit 成功取得 remote job id 后，后续恢复应继续 poll 同一 job，不重新 submit。

## 11. 安全日志、诊断与敏感字段遮蔽

### 11.1 普通日志允许字段

采用 allowlist，而不是先记录再正则清洗。允许：

- 本地 `taskId`、`operationId`、`attemptId`；
- provider/adapter 标识与版本；
- 阶段、attempt、耗时、HTTP status、稳定错误码；
- MIME、音频字节数、时长等非正文元数据；
- 经审核的远端 request id；
- retry/cooldown 的毫秒数。

文件名、绝对路径、会议标题也可能包含敏感信息；普通日志只记录 `AudioArtifactRef.id`，不记录这些值。

### 11.2 永不记录的内容

- API Key、Token、Cookie、credential value、Authorization/Proxy-Authorization；
- request/response headers 全量 dump；
- endpoint query、userinfo、内部地址；
- request/response body、multipart boundary/body、音频字节；
- transcript、segment text、meeting minutes、prompt、用户提供的参会人/标题；
- SecretStore/Windows Credential Manager 错误中的 secret value；
- 会包含以上内容的 panic/debug representation。

### 11.3 Redaction 规则

- 敏感 header 的值统一输出 `[REDACTED]`；生产日志更推荐完全不输出该 header。
- URL 日志最多保留公开允许的 scheme、host 与 adapter route label；删除 userinfo、query 和 fragment。内部 endpoint 默认只记录 provider id。
- JSON/body 不进入普通日志，因此不能依赖递归 redaction 来“安全记录 body”。
- 配置 dump 中键名匹配 `key`、`token`、`secret`、`password`、`credential`、`authorization`、`cookie` 时值必须删除；仍以配置 allowlist 为主。
- provider/transport 错误必须先映射为 `ProviderError` 再进入 IPC；禁止直接格式化底层 request 对象。
- 建议 credential 使用不可意外 `Debug`/`Display` 的 secret wrapper，并在生命周期结束时尽力清零内存；这不能替代系统凭据管理。

### 11.4 安全测试哨兵

自动测试使用明显但无效的 sentinel secret 和 sentinel transcript。测试结束后扫描内存外可见产物：日志、IPC fixture、snapshot、错误消息与测试报告，必须没有原值；Authorization 应完全不存在或仅为 `[REDACTED]`。测试值也不得模仿真实有效 key 格式或提交真实凭据。

## 12. Mock Provider 契约

Mock 必须完全离线、确定性、可取消，并使用与真实 provider 相同的内部 traits。它不能通过“特殊 UI 分支”绕开 provider 契约。

### 12.1 成功行为

- mock ASR 只接收 ImportService 生成的 `AudioArtifactRef`，校验只读引用仍有效、非空且与 staging metadata 一致，但不尝试真实识别，也不把音频内容读入日志。
- mock ASR 返回仓库内固定、非敏感的 fixture transcript；返回内容不应从本地测试音频文件的真实会议内容提取。
- mock minutes 返回由 Agent 5 Schema 校验通过的固定 fixture，并记录正确的 schema/template version。
- mock 调用历史只记录 task/operation/attempt、scenario、阶段、耗时和安全元数据。

### 12.2 可注入场景

| 场景 id | 预期行为 |
| --- | --- |
| `success` | ASR 与 minutes 均成功 |
| `delay` | 可配置延迟；等待期间可取消 |
| `timeout_connect` | 模拟 body 未发送的连接超时 |
| `timeout_after_send` | 模拟发送后结果未知；验证无幂等时不自动重试 |
| `http_401` / `http_403` | 认证/权限失败，0 次自动重试 |
| `http_429_then_success` | 前 N 次 429，带安全的 Retry-After，随后成功 |
| `http_500_then_success` | 前 N 次 500，按 replay-safety 验证 retry 次数 |
| `network_unavailable` | 临时网络错误 |
| `malformed_response` | 非法 JSON 或字段形状错误 |
| `oversized_response` | 超过 response size limit，停止读取 |
| `empty_transcript` | ASR 返回空白；minutes 调用次数必须为 0 |
| `invalid_minutes_schema` | 纪要结果未通过 Schema，不得完成任务 |
| `cancel_upload` / `cancel_poll` / `cancel_generate` | 在各活动阶段取消并忽略 late success |
| `concurrency_429` | 多任务触发共享 cooldown，验证批量公平性 |

场景由显式 mock 配置选择，不从文件名、会议标题或正文触发。延迟和失败次数使用确定性参数或虚拟时钟，避免脆弱测试。

### 12.3 Mock 调用断言

测试可读取仅限测试构建的安全调用记录：

```text
MockCallRecord {
  operationId
  attemptId
  operationKind
  scenario
  startedAt
  completedAt?
  outcome
}
```

记录不得含用户源文件路径、音频 bytes、transcript、minutes、prompt、credential 或 HTTP body。

## 13. Phase 1 Mock 验收标准

Phase 1 不要求真实 API。下列项目必须有实际命令和退出码证据，未运行项标记 BLOCKED：

1. `TranscriptionProvider`、`MinutesProvider`、能力 DTO 与 `ProviderError` 在 Rust 中可编译，并有函数级文档注释。
2. 无 Key、无网络时，mock `success` 能完成 `ImportService 只读校验 -> AudioArtifactRef/staging metadata -> Transcript -> MinutesCandidate`；再经纪要模块唯一 validator 生成 schema-valid `MeetingMinutesEnvelope`。
3. `empty_audio` 在发起 provider 调用前失败；`empty_transcript` 使 minutes 调用次数为 0。
4. 401/403 不自动重试；429 尊重有上限的 Retry-After；replay-safe 的 500/timeout 不超过 `1 + maxRetries`。
5. `timeout_after_send` 在没有已验证幂等能力时只返回 `outcome_unknown`，不自动重复提交。
6. 在批量排队、retry wait、上传、转写/poll 和总结阶段执行单文件取消或整批取消时，对应任务最终均为 `cancelled`，late success 不写入结果，用户源文件保持不变。
7. 同一 dedupe key 的并发提交只产生一个活动 operation；新用户重跑能建立新 operation。
8. response size limit、malformed response 和 invalid Schema 均产生稳定安全错误，原始 body 不穿过 IPC。
9. endpoint/timeout/retry/concurrency 配置校验覆盖边界值；生产 HTTP 和带 userinfo 的 URL 被拒绝，loopback HTTP 只在显式开发模式允许。
10. sentinel secret、Authorization 和 sentinel transcript 不出现在日志、错误、snapshot、mock call record 或测试报告中。
11. mock fixture 不包含仓库本地测试音频文件的真实会议内容；真实离线音频只作为 ImportService 输入验证。
12. task 状态事件只含阶段和安全元数据，不伪造百分比，不含正文。

Phase 1 完成报告必须明确写出：执行的 `cargo test`/集成测试/类型检查/构建命令、退出码、测试数、失败数和未执行原因。仅生成文件不算通过。

## 14. Phase 4 真实 API 验证与响应记录

### 14.1 验证前提

- 真实 Key 只能通过环境变量或 Windows Credential Manager 注入；PowerShell 命令不得回显值，不使用会打印 header/body 的 `curl -v` 或 HTTP trace。
- 先使用最小、非敏感测试音频；记录 MIME、字节数和时长，不记录文件名、绝对路径、哈希或转写正文。
- endpoint 必须来自用户明确配置或供应商官方资料。内部 endpoint 不写进仓库文档。
- 真实 adapter 初始状态为 `Unverified`；只有成功与必要失败路径均有证据后，才更新对应 capability。

### 14.2 安全的验证记录格式

每个 provider/adapter/operation 建立一份脱敏验证记录，建议未来位于 `docs/provider-contracts/<adapter-id>/<date>-<operation>.md`。公开供应商可记录公开名称；企业内部服务使用匿名 provider id。记录至少包含：

```text
VerificationRecord {
  verifiedAtUtc
  testerEnvironment: Windows version + app commit/build id
  adapterId + adapterVersion
  operation: transcription | minutes
  providerLabel: public name or anonymized id
  documentationReference?: public official URL + document date/version
  requestShape: {
    method
    routeLabel                 # 不记录内部完整 URL/query
    contentType
    authStrategyLabel          # 只写策略名，不写 header/value
    modelLabel                 # 敏感内部模型则匿名
    audioMimeType?
    audioByteLength?
    audioDurationMs?
  }
  observedResponse: {
    httpStatus
    contentType?
    responseByteLength
    structuralShape           # 只记录字段路径、类型、是否可选/数组基数
    remoteRequestIdAvailable: boolean
  }
  lifecycle: sync | async-submit-poll
  observedErrors: status -> stable ProviderError mapping
  retryAfterObserved?: delta-seconds | http-date | absent
  idempotencyEvidence?: documentation/observed behavior summary
  cancellationEvidence?: local-only | remote-confirmed | unknown
  result: verified | partially_verified | blocked
  limitations
}
```

`structuralShape` 只记录结构，例如 `result.segments: array<object>`、`segment.start: number`，所有 string/number 实际值均用类型和长度/基数代替。不得保存 raw request/response、转写片段、纪要正文、prompt、header 值或音频。

### 14.3 映射证据表

真实 codec 的文档必须附映射表，至少包含：

| 内部字段 | provider 字段路径 | 请求/响应 | 必需性 | 转换/单位 | 证据来源 | 状态 |
| --- | --- | --- | --- | --- | --- | --- |
| 示例仅表示格式，不预设字段 | Phase 4 实测后填写 | - | - | - | 官方文档/脱敏实测 | unverified |

- 每个映射项必须有官方文档或脱敏实测证据。
- 单位转换必须显式记录，例如秒到毫秒；未确认单位不得猜测。
- 缺失字段的行为要记录为“省略/空/错误”，不能用默认值掩盖。
- 如实记录同步/异步、提交/轮询终态、大小限制、429、5xx、取消与幂等差异。

### 14.4 Phase 4 最小验证矩阵

1. 最小有效音频成功一次，记录状态、结构和耗时，不记录正文。
2. 使用无效测试凭据或受控方式验证 401/403；不得锁定真实账户或把凭据写日志。
3. 仅在安全且供应商允许时验证 429/5xx；无法触发则记为未验证，不伪造。
4. 验证实际 MIME、大小/时长上限来源、同步/异步生命周期和 polling 终态。
5. 验证取消是只停止本地等待还是能远端确认取消。
6. 验证 idempotency key 是否支持、作用域和重复请求语义；未验证则 `NeverAutomaticallyReplay` 或更保守策略。
7. minutes 成功结果通过 Agent 5 Schema；不通过时记录结构差异并修正 codec/prompt，不把 raw body提交仓库。
8. 运行日志 sentinel 扫描，确认无 Key、Authorization、音频内容、transcript 或 minutes。

若真实 API 不可访问、字段不明或凭据不可用，结果必须写 `BLOCKED`/`unverified`，继续保留 mock provider，不得根据相似供应商接口伪造成功。

## 15. 跨 Agent 接口约束

- Agent 2/UI：只消费 Lead 暴露的稳定 IPC DTO、阶段和 `ProviderError.safeMessage`；不得直接发云请求或读取密钥。
- 离线导入模块负责人：ImportService 只读校验用户选择的文件，并向 provider 层交付 `AudioArtifactRef`；只有受信任后端可解析其 id，前端与 provider DTO 均不暴露路径。
- Agent 5/纪要：维护唯一 JSON Schema、模板版本与 parser/validator；provider 只引用 schema version 和验证器，不复制 Schema。
- Agent 6/安全 QA：应覆盖 sentinel 泄露扫描、unknown-outcome 重试、取消竞态、429 shared cooldown、response size limit 和异步远端取消语义。
- Lead：统一配置、SecretStore、task dedupe、SQLite 状态、IPC 类型和 provider 模块注册；不同 Agent 不并发修改入口文件。

## 16. Phase 2 实现状态与未决项

Phase 2 已在 `src-tauri/src/providers/**` 实现：

- `TranscriptionProvider`、`MinutesProvider`、稳定 DTO、`ProviderError`、能力与 replay-safety 类型；
- Provider 直接复用 ingest 的 `AudioArtifactRef/StagingMetadata`，不维护重复 DTO；`ManagedAudioArtifact` 只增加 `Arc<dyn AudioArtifactReader>`，每次 attempt 通过 ingest 注册表重新打开只读 handle，Provider 不持有或输出 staged path；
- 不可 serde、`Debug` 永远遮蔽且 drop 时尽力清零的 `ProviderCredential`；Key 只能由可信调用方在调用时传入；
- 自定义可克隆取消令牌，覆盖并发排队、最小请求间隔、HTTP、retry wait、响应读取和总 deadline；
- redirect-disabled `ReqwestHttpExecutor`，流式 multipart 文件上传、有限响应读取、鉴权注入与安全错误映射；
- `HttpExecutor` trait 与脚本化测试 executor，使 401、429、5xx、网络、取消和协议响应无需真实 Key 即可验证；
- 确定性 `MockProvider`，调用记录只包含 operation/artifact ID、场景、时间和 outcome；
- OpenAI-compatible transcription codec：multipart 字段、toggle 值、通用字段、上传文件名、响应 JSON path、segment path、时间单位与 request-id header 均必须显式配置；
- OpenAI-compatible minutes codec：完整 JSON body template、model/prompt/schema 精确占位符、响应 JSON path 与 `JsonValue`/`JsonEncodedString` 模式均必须显式配置；
- bounded exponential backoff + deterministic full jitter、`Retry-After` 上限、401/403 终止、429/5xx 条件重试、并发 semaphore 和共享 cooldown；
- operation id 经 SHA-256 生成 header-safe 幂等值，且只在显式配置并声明已验证 replay-safety 时发送。

当前真实 HTTP adapter 只实现同步 request-response。若 capabilities 声明异步 job 或远端撤销，构造器会拒绝配置；本地取消只保证停止等待、上传/读取和结果保存。未来异步 submit/poll/cancel 必须用独立、经验证的 codec 扩展，不能复用当前同步实现伪称支持。

2026-07-17 已根据官方文档固化设置预设，但不把文档核对冒充真实互操作测试：

- `dashscope_funasr_cn`：提交端点 `https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription`，默认模型 `fun-asr`，可选 `fun-asr-mtl`；
- `dashscope_funasr_intl`：提交端点 `https://dashscope-intl.aliyuncs.com/api/v1/services/audio/asr/transcription`，模型同上；
- `deepseek`：请求端点 `https://api.deepseek.com/chat/completions`，默认模型 `deepseek-v4-flash`，可选 `deepseek-v4-pro`；
- `xiaomi_mimo_llm`：请求端点 `https://api.xiaomimimo.com/v1/chat/completions`，默认模型 `mimo-v2.5`，可选 `mimo-v2.5-pro`；该地址与 MiMo ASR 共用，后端必须结合模型和业务目标推断预设；
- 托管预设的地址由可信后端解析并校验模型白名单，普通 UI 不允许编辑；`custom_openai_compatible` 显示自定义完整 Chat Completions 地址和模型。

百炼 Fun-ASR 不是 OpenAI multipart 转写接口。它需要“本地文件临时上传或 OSS → 异步提交 → 按 `task_id` 轮询 → 下载 `transcription_url` 结果”的专用 adapter；签名 URL、临时上传凭据和策略均视为秘密运行时数据，不得写入普通日志或 SQLite。

以下事项仍保持未决，必须在 Phase 4 用证据关闭：

- 百炼 Fun-ASR 临时上传/企业 OSS 策略、轮询终态、结果下载允许域、响应归一化、时间戳/说话人字段和音频限制；
- DeepSeek JSON Output 与现有 MeetingMinutes Schema validator 的真实响应互操作、token 限制和错误结构；
- 各 provider 的鉴权策略、Retry-After 行为、远端取消和 idempotency 支持；
- 企业代理、自签 CA 与内部 endpoint 的安全配置；
- response 大小、timeout、retry、并发的具体默认值。

这些未决项不阻塞 mock-first 集成，但阻塞把 FunASR 或任何真实 adapter 标记为已验证、可安全自动重试或支持远端撤销。

## 17. Phase 2 验证记录

2026-07-17 在 Windows `stable-x86_64-pc-windows-msvc` 实际执行：

| 命令 | 结果 |
| --- | --- |
| Provider 文件 `rustfmt --edition 2021 --check` | 通过 |
| `cargo check` | 通过，无 Provider warning |
| `cargo test providers -- --nocapture` | 通过，19 passed、0 failed；另有 18 项非 Provider 测试被过滤 |
| `cargo clippy --lib --tests -- -D warnings` | 通过 |
| `cargo test --all-targets` | 通过，37 passed、0 failed |
| `cargo build` | 通过，dev profile |

Provider 测试覆盖：mock 完整候选流程与取消、空转写、401 不重试、429 后成功、5xx 有限重试、发送前网络错误重试、HTTP executor 强制取消、非法响应、显式 minutes 模板/响应路径、重放安全、Retry-After 上限、endpoint 校验，以及 credential/transcript/segment/minutes/response body 的 Debug/错误遮蔽。

当前没有调用真实 API，也没有独立进程 mock server 或 `src-tauri/tests` 外部集成测试；HTTP 行为通过公开 `HttpExecutor` 边界和 scripted executor 验证。真实 multipart 网络互操作、企业代理、自签 CA、真实状态/响应结构与远端取消仍属于 Phase 4 验证项。

## 18. Xiaomi MiMo 与火山引擎录音文件 ASR 契约（2026-07-20）

本节只记录 2026-07-20 从厂商官方文档核对并在本地 scripted executor 中固化的字段。没有使用真实 Key 调用，不把 contract test 记为真实互操作测试。

### 18.1 Xiaomi MiMo V2.5 ASR

官方来源：

- [Speech Recognition API Reference](https://platform.xiaomimimo.com/static/docs/api/audio/Speech-Recognition.md)
- [Speech Recognition Usage Guide](https://platform.xiaomimimo.com/static/docs/usage-guide/Speech-Recognition.md)
- [Model and Rate Limit](https://platform.xiaomimimo.com/static/docs/quick-start/model.md)

已固化事实：

- 端点：`POST https://api.xiaomimimo.com/v1/chat/completions`；模型固定为 `mimo-v2.5-asr`。
- 鉴权支持 `Authorization: Bearer` 或 `api-key`；托管预设选择 Bearer，Key 只由 HTTP transport 注入。
- 请求使用 `messages[0].content[0].input_audio.data`，值为 `data:{MIME_TYPE};base64,...`；只允许单个 MP3/WAV。
- Base64 编码后的 data URL 不得超过 10 MB。适配器按十进制 10,000,000 字节做保守预检，对最长 MIME 前缀计算出的原文件上限为 7,499,982 字节。
- `asr_options.language` 只允许 `auto`、`zh`、`en`；非流式文本位于 `choices[0].message.content`，请求 ID 取响应 `id`。
- 官方响应未定义中立契约所需的分句时间戳、说话人或置信度，因此这些能力保持 `false`，不得伪造。

### 18.2 火山引擎录音文件识别极速版

官方来源：

- [录音文件极速版识别 HTTP](https://www.volcengine.com/docs/6561/1631584)
- [录音文件识别标准版 HTTP](https://www.volcengine.com/docs/6561/1354868)（字段与 URL 模式交叉核对）

选择极速版是因为它面向非实时录音文件，单次请求直接返回结果，不需要 submit/query 轮询；同时支持本地文件 Base64 和 Provider 侧拉取 URL，符合本项目的文件转写边界。

已固化事实：

- 端点：`POST https://openspeech.bytedance.com/api/v3/auc/bigmodel/recognize/flash`；`request.model_name=bigmodel`。
- 新版控制台鉴权使用单个 `X-Api-Key`；资源 ID 固定为 `volc.bigasr.auc_turbo`。适配器还发送随机操作 ID 对应的 `X-Api-Request-Id` 和固定 `X-Api-Sequence: -1`。
- 本地文件使用 `audio.data=<raw base64>`；URL 文件使用 `audio.url=<validated https URL>`，二者均发送明确 `audio.format`。
- 极速版文档限制为最长 2 小时、最大 100 MB，格式为 WAV/MP3/OGG OPUS。已知本地元数据和 URL 元数据会在请求前校验；URL 元数据未知时不伪装成已验证。
- 业务成功码来自响应 Header `X-Api-Status-Code: 20000000`，远程诊断 ID 只保留 `X-Tt-Logid`；响应正文全文位于 `result.text`，分句位于 `result.utterances`，时间单位为毫秒。
- `550xxxx` 映射为可重试的 Provider 故障，但由于未验证请求发送后的幂等性，adapter 内不会自动重放已发送请求；由上层显式重试。
- 旧控制台需要 App ID + Access Token 两个值，当前秘密槽位只有一个 Key，因此托管预设明确只支持新版控制台，不复用或拼接双凭据。

### 18.3 URL 安全边界

`RemoteAudioFile` 只接受 HTTPS，不允许 URL 用户名、密码或 fragment，长度上限为 8192。预签名 URL 可以保留 query，但整个 URL 在 `Debug`、错误和普通日志中始终显示为 `[REDACTED]`。应用不在本地抓取该 URL；支持 URL 的 Provider 将它作为敏感请求正文交给供应商拉取。

`TranscriptionCapabilities.supportsRemoteUrls` 明确区分 Provider 是否支持 URL。mock provider 已覆盖 URL 流程；火山引擎为 `true`，MiMo 和通用 multipart adapter 为 `false`。当前稳定 IPC 和桌面 UI 仍只接受用户选择的本地文件，URL 输入的前端类型、持久化生命周期和任务创建命令需要单独阶段验收后开放。

## 19. MiMo LLM 与 OpenAI 兼容纪要协议（2026-07-21）

本节基于 Xiaomi MiMo 官方模型页面和 OpenAI 兼容字段约定固化请求边界，只使用 scripted executor 验证，不包含真实 Key 或真实会议原文。

官方来源：

- [Xiaomi MiMo 模型站](https://mimo.xiaomi.com/zh)
- [MiMo-V2.5 模型说明](https://mimo.mi.com/docs/zh-CN/model-intro/mimo-v2.5)
- [MiMo-V2.5-Pro 模型说明](https://mimo.mi.com/docs/zh-CN/model-intro/mimo-v2.5-pro)

已固化事实与兼容边界：

- MiMo 纪要预设使用 `POST https://api.xiaomimimo.com/v1/chat/completions`，允许 `mimo-v2.5` 和 `mimo-v2.5-pro`；托管地址和模型白名单不能由前端覆盖。
- Chat Completions 请求使用 `model + messages[0].role/content`，响应只读取 `choices[0].message.content`。
- 两种响应内容都按“不受信任的 JSON 字符串”解析，随后必须通过唯一的 MeetingMinutes Schema 与语义校验器；缺字段、非 JSON、版本不符或证据引用无效均失败，不自动修造纪要。
- 通用第三方预设不推测上下文长度、JSON Schema、远程取消、幂等或自动重放能力；网络请求仍受 HTTPS、超时、有限响应、取消和安全错误分类约束。
