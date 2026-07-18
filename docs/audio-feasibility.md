# 离线音视频导入与校验可行性

> 状态：Phase 0 设计结论，不代表导入功能已经实现
> 日期：2026-07-17
> 适用范围：Windows 11 x64；只处理用户已存在的本地 WAV、MP3、M4A、MP4、MOV 文件
> 本文不定义任何真实 Provider 的私有 HTTP 字段；Provider 契约以 [API 契约](./api-contract.md) 为准。

## 1. 结论与范围

离线文件导入在 **Tauri 2 + Rust** 中实现难度低，不需要额外硬件访问，也不需要把整段音频载入前端或内存。

MVP 建议：

- 使用 Tauri 官方 dialog plugin 提供 Windows 原生单选/多选文件对话框，过滤器提示 `wav`、`mp3`、`m4a`、`mp4`、`mov`。官方文档确认 `multiple` 控制多选，且 Windows 返回文件系统路径。[Tauri Dialog](https://v2.tauri.app/plugin/dialog/)
- 文件对话框由 Rust Core 发起或由 Lead 提供一次性受控导入句柄；前端不获得可长期复用的任意文件读取能力。
- 用户源文件始终只读。应用处理的是复制到 app-local-data 下的受管 staging 副本，取消、失败、删除会议均不得修改或删除外部源文件。
- 扩展名和 Windows MIME 只作为提示。最终格式由文件头、容器结构、音频轨和解码检查共同判定。
- Phase 1 可固定评估 `symphonia = "=0.6.0"`，只启用 `wav`、`pcm`、`mp3`、`isomp4`、`aac`、`alac` 所需 feature。它是纯 Rust demux/decoder，官方列出 Wave、ISO/MP4、MP3、PCM、AAC-LC 和 ALAC 支持。[Symphonia 0.6.0](https://docs.rs/symphonia/0.6.0/symphonia/)
- 不把开发机上的 FFmpeg 作为客户端运行依赖。FFmpeg/ffprobe 只用于生成非敏感 fixture 和交叉验证 PoC 结果。
- 默认不转码。真实 Provider 的已验证 capability 若不接受原文件，再单独评估 adapter 内转码、分片或临时对象存储；Phase 0 不提前引入 FFmpeg sidecar。

`symphonia` 0.6.0 发布较新且采用 MPL-2.0，进入生产依赖前必须完成 Windows fixture、包体、性能和许可证复核。若其 M4A 兼容性不足，保持 `OfflineAudioImporter` 接口不变，再比较 Windows Media Foundation 或受控 sidecar；不能把 Phase 0 推荐写成已验证事实。

## 2. 用户选择与批量语义

### 2.1 单文件和批量使用同一后端流程

```text
系统文件对话框
  -> 0..N 个用户选择项
  -> 每项独立快速检查
  -> 只读流式复制 + SHA-256 到 staging .part
  -> staging 副本结构/解码校验
  -> Provider capability preflight
  -> artifact ready / duplicate / failed / cancelled
```

- 单文件是批量大小为 1 的特例，不维护两套校验逻辑。
- 用户取消系统对话框返回空选择，不是错误，也不创建任务。
- 文件过滤器不能作为安全边界；后端仍须拒绝目录、不支持的扩展名、伪扩展名和损坏文件。
- 批量逐项返回结果。一个文件失败不回滚已成功的其他文件，也不能把整个批次显示为“全部成功”。
- `maxBatchItems`、`maxBatchTotalBytes` 和本地校验并发数必须配置化并使用 checked arithmetic；不能把所有文件内容同时读入内存。
- 建议磁盘型校验默认低并发，Phase 1 从 2 个 worker 起测；最终默认值由 Lead 根据机械硬盘、SSD 和企业终端实测固化。
- 文件名、绝对路径属于敏感元数据。文件名可在受控 UI 中显示，但普通日志、遥测、错误报告和 Provider metadata 只能使用 artifact/operation ID。

### 2.2 路径边界

- Rust 内部使用 `PathBuf`/Windows 原生路径语义，不用字符串拼接，不假设路径是 ASCII。
- 用户可能选择本地盘、移动盘、UNC 或云同步占位文件。PoC 应记录这些来源的行为；不能在没有证据时承诺全部支持。
- 不把用户选择路径保存为长期处理来源。复制成功后，后续校验、上传、重试只读取受管 artifact。
- 不根据原文件名生成 staging 文件名；使用随机 opaque artifact ID 和由探测结果确定的规范扩展名。
- 导入服务不接受前端传入的任意“目标 staging 路径”。受管根由 Rust Core 解析，前端最多持有 artifact ID。

## 3. 格式识别与可读性校验

### 3.1 四层校验

| 层级 | 检查 | 目的 |
| --- | --- | --- |
| 1. 文件系统 | 存在、普通文件、可只读打开、字节数非零、checked size limit | 快速拒绝目录、消失、不可读、空或明显超限文件 |
| 2. 扩展名/头部 | 扩展名白名单 + bounded header sniff | 快速识别明显伪装；不据此宣布文件有效 |
| 3. 容器/音轨 | demux、至少一个 audio track、codec/sample rate/channels/duration 合理 | 获得可信技术元数据，拒绝无音轨或结构损坏 |
| 4. 解码 | 流式解码完整音频轨并丢弃样本；内存有界且可取消 | 捕获后段截断、无可解码帧和潜在损坏 |

只读取一个 magic number 无法证明压缩媒体完整可用。当前实现对 WAV、MP3 和 ISO BMFF 执行有界结构校验；MP4/MOV 还会遍历 `moov/trak/mdia/minf/stbl/stsd`，要求恰有一条 AAC 或 ALAC 音轨。当前并未完整解码整个文件，因此后段码流损坏仍可能由云端 Provider 才发现，这是已知限制。

若完整解码成本在 Phase 1 soak 中不可接受，可把“快速 probe + 后台完整校验”拆为两个阶段，但 `ready` 只能在完整校验通过后出现。

### 3.2 支持矩阵

| 用户格式 | 扩展名提示 | 快速头部提示 | MVP 允许的容器/codec | 内部 MIME 建议 |
| --- | --- | --- | --- | --- |
| WAV | `.wav`，大小写不敏感 | `RIFF....WAVE`；`RF64` 仅在解析器实测支持后开放 | RIFF/WAVE + PCM；其他 WAV codec 需 capability 和 decoder 证据 | `audio/wav` |
| MP3 | `.mp3` | `ID3` 或 MPEG audio frame sync；单独 `ID3` 头不算有效音频 | MPEG audio + MP3，至少一个可解码 frame | `audio/mpeg` |
| M4A | `.m4a` | ISO BMFF box，常见 `ftyp`；不能只匹配某一个 brand | ISO/MP4 + AAC-LC 或 ALAC | `audio/mp4` |
| MP4 | `.mp4` | ISO BMFF `ftyp`/`mdat`/`moov` | 视频轨可选；必须恰有一条 AAC 或 ALAC 音轨 | `video/mp4` |
| MOV | `.mov` | ISO BMFF/QuickTime `ftyp`/`mdat`/`moov` | 视频轨可选；必须恰有一条 AAC 或 ALAC 音轨 | `video/quicktime` |

规则：

- 扩展名与探测容器不匹配时返回 `extension_content_mismatch`，不静默改名上传。
- 容器受支持但 codec 不在允许矩阵时返回 `unsupported_audio`；视频没有音轨时返回 `missing_audio_track`。
- M4A 是容器习惯扩展名，不等于 codec；必须分别记录 container 与 codec。
- MIME 是应用内部规范值；真实 adapter 如需不同 wire `Content-Type`，由经验证的 adapter 映射，不能污染导入模块。
- 只使用主音频轨。多音轨选择策略不是 Phase 0 已解决事实；MVP 可明确拒绝多音轨并返回 `unsupported_audio_tracks`，避免静默选错会议语言/轨道。
- sample rate、channel count、duration 和 decoded frame count 使用非负 checked integer；零帧或零时长返回 `empty_audio`。

### 3.3 Symphonia PoC 边界

建议 Phase 1 只启用需要的 feature，不使用 `all`，避免引入无关格式和 codec。PoC 必须验证：

- PCM WAV、MP3、M4A/AAC、M4A/ALAC（如果能生成确定性 fixture）；
- ID3v2 MP3、无标签 MP3、不同合法采样率/声道；
- truncated header、truncated tail、无音轨 ISO/MP4、错误扩展名；
- 现有真实 MP3 的 duration 与 ffprobe 交叉差异；
- M4A duration 优先由实际解码帧数/采样率计算，不只信容器声明。

Phase 1 应把版本精确锁进 `Cargo.lock`。如果 0.6.0 对目标 fixture 不稳定，应记录具体文件的非敏感技术特征和错误类型，不记录文件内容、路径或标签。

## 4. 大小、时长与资源限制

不存在适用于所有 Provider 的统一文件上限，因此分为两层：

1. **本地安全上限**：`import.maxFileBytes`、`maxBatchItems`、`maxBatchTotalBytes`，防止磁盘/内存/整数滥用。生产默认值由 Lead 在 Phase 1 性能与磁盘策略确定，本文不虚构数值。
2. **Provider 上限**：来自 `TranscriptionCapabilities.maxAudioBytes` 与 `maxDurationMs`。缺失表示未知，不得用任意大数冒充已验证支持。

处理规则：

- 文件 metadata 的 byte length 超过任一已知硬上限时，在复制和网络请求前失败。
- duration 只能在可信 probe/解码后判断；超过 Provider 时删除未提交的 staging 或按保留策略登记，不发网络请求。
- 读取、复制、哈希、解码和上传都使用流式接口；禁止 `read_to_end` 处理整个音频。
- copy buffer、decoder packet、队列和并发均有上限。批量总字节数用 checked addition，溢出按超限失败。
- 启动复制前检查 staging volume 可用空间；检查只能降低风险，仍必须处理复制过程中的 `disk_full`。
- 取消必须传播到 copy/hash/decode；取消后关闭 handle 并清理该 item 的 `.part`，不能留下可上传的完成 artifact。
- Phase 1 边界测试注入 1 MiB 单文件上限和小批次数量，不代表生产默认或任何 Provider 实际限制。

## 5. Staging 与用户源文件保护

### 5.1 写盘协议

```text
external source (read-only handle)
  -> app-local-data/audio-staging/<random-id>.part
  -> stream copy + SHA-256 + byte count
  -> flush/sync
  -> probe + full decode from staging copy
  -> capability preflight
  -> atomic rename within managed root to <artifact-id>.<canonical-ext>
  -> persist AudioArtifactRef
```

- 只通过 read-only `File::open` 或等价 `OpenOptions.read(true)` 打开源文件；Rust 标准库的 `File::open` 即为只读打开。[Rust `std::fs::File`](https://doc.rust-lang.org/std/fs/struct.File.html)
- 复制前后比较打开 handle 的 size/mtime 等稳定元数据；复制字节数必须等于初始 byte length。发现变化返回 `source_changed_during_import` 并删除 `.part`。
- 对真正的竞态保护以“处理的永远是已经复制并哈希的 staging bytes”为准；不能继续从外部路径上传。
- staging 根不在仓库、当前目录、通用 `%TEMP%` 或共享目录。PoC 可使用 `.artifacts` 隔离目录，但生产必须使用 Tauri app-local-data。
- 只有同一受管目录内完成 sync 后才执行 rename；失败保持 `.part`/cleanup pending，不写完成记录。
- 启动清理只删除 manifest/数据库证明归本应用所有且过期的 `.part`，禁止宽泛 glob 删除用户文件。
- 删除会议或取消任务只处理 managed copy；`external_source` 永不由应用删除。
- 不长期保存原始绝对路径。若业务需要在 UI 显示原文件名，只存长度受限的 display name，并按敏感数据保护。

### 5.2 只读保护验收

至少用一个带 ReadOnly 属性的非敏感生成音频验证成功导入，并在导入、取消、去重、删除 artifact 后比较源文件：

- size 未变化；
- SHA-256 未变化；
- `LastWriteTimeUtc` 未变化；
- ReadOnly 属性仍存在；
- 源文件仍存在且可由独立工具解析。

源文件哈希仅在测试进程内比较，不输出到普通日志或报告。

## 6. 哈希、去重与幂等

- 流式复制时同时计算 SHA-256，避免为了去重再完整读取一次源文件。
- 完整 SHA-256 可保存在受保护数据库和 `AudioArtifactRef.sha256`，但不得进入普通日志、遥测、错误文本或 UI。
- 去重对象是 **managed audio artifact**，不是业务任务：相同 bytes 只保留一份受管副本，但用户仍可用不同模板/配置创建新的处理任务。
- 两次导入 SHA-256 和 byte length 相同，返回同一个 artifact ID 或增加引用，不再复制第二份文件。
- 同一 artifact + 同一处理配置的并发重复提交由 task orchestrator 做活动任务幂等，不能由路径或文件名推断。
- 同一路径内容发生变化会得到新 hash，因此是新 artifact。
- 批量并发去重需要数据库唯一约束/事务，避免两个 worker 同时写出两份；临时输家副本按 manifest 安全清理。
- Phase 1 使用生成 fixture 验证哈希逻辑；现有真实 MP3 不把 hash 写入测试输出、snapshot 或 Agent 报告。

## 7. Provider capability preflight

导入模块不定义供应商字段。它只消费 [API 契约](./api-contract.md) 已有的：

```text
TranscriptionCapabilities {
  evidence
  acceptedMimeTypes
  maxAudioBytes?
  maxDurationMs?
  ...
}
```

preflight 顺序：

1. 本地 source 快速大小/类型检查；
2. staging copy + authoritative probe/decode；
3. 生成 `AudioArtifactRef { id, mimeType, byteLength, durationMs, sha256 }`；
4. 与当前 adapter 的 `acceptedMimeTypes`、`maxAudioBytes`、`maxDurationMs` 比较；
5. 成功后才能创建/推进网络处理任务。

规则：

- `evidence: Mock` 只供 mock 流程；`Unverified` 不得宣称真实 Provider 可处理该文件。
- MIME 不接受、超字节或超时长都在网络前返回 `unsupported_audio` 或更具体的本地 preflight code，Provider 调用次数必须为 0。
- capability 缺失的数值上限表示未知；Phase 4 必须用官方契约或最小非敏感文件验证后更新。
- 导入可以在未配置 Provider 时完成本地 artifact 校验，但处理任务进入 `provider_not_configured`，不能伪装为可上传。
- 是否 multipart、异步任务、公开 URL 或其他上传方式属于 adapter 合同，不由导入模块猜测，也不触发默认转码。
- 若真实 adapter 只接受另一格式，先记录 capability 差异，再评估 adapter 内转换。源 artifact 和转换 artifact 必须分开建模、分别哈希和清理。

## 8. 错误与批量结果契约

### 8.1 建议错误码

| 错误码 | 场景 | 网络调用 |
| --- | --- | --- |
| `source_not_found` | 选择后文件消失 | 0 |
| `source_not_file` | 目录或不允许的对象 | 0 |
| `source_unreadable` | 只读打开/读取失败 | 0 |
| `source_changed_during_import` | copy 前后元数据/字节数不一致 | 0 |
| `empty_audio` | 零字节（0 bytes）、零帧或零时长 | 0 |
| `file_too_large` | 超本地或 Provider byte limit | 0 |
| `batch_limit_exceeded` | 数量/总字节超本地策略 | 0 |
| `extension_content_mismatch` | 扩展名与探测容器不一致 | 0 |
| `corrupt_audio` | demux/decode 失败、尾部截断、无可解码帧 | 0 |
| `unsupported_audio` | container/codec/MIME/duration 不被本地或 Provider 接受 | 0 |
| `audio_storage_failed` | staging 创建、写、sync、rename 失败 | 0 |
| `cancelled` | 用户取消 copy/probe/decode | 0 |

这些是导入/本地 preflight 语义；进入 Provider 层时应映射到 `api-contract.md` 的统一错误模型，不把底层 parser、Windows 路径或 OS 错误全文发给 UI。

### 8.2 建议返回形状

```text
ImportBatchResult {
  batchId
  items: ImportItemResult[]
}

ImportItemResult {
  selectionIndex
  displayName?              # 敏感，仅受控 UI；不进日志
  status: Ready | Duplicate | Failed | Cancelled
  artifact?: AudioArtifactRef
  duplicateOfArtifactId?
  error?: SafeTaskError
}
```

返回值和事件都不包含 source path、managed path、音频 bytes、parser dump 或完整 hash。进度只报告 `checking`、`copying`、`validating`、`preflight`、`ready` 等真实阶段；没有精确字节进度时不伪造百分比。

## 9. 本机 Phase 0 证据

2026-07-17 对仓库根目录现有 MP3 只执行了文件系统 metadata 和 ffprobe 技术字段查询，命令未请求 tags、正文或转写：

| 字段 | 实际结果 |
| --- | --- |
| 文件类型 | 普通 `.mp3` 文件 |
| byte length | `31,185,261` |
| container / codec | MP3 / MP3 |
| duration | `1,949.076` 秒，约 32 分 29 秒 |
| sample rate | `16,000` Hz |
| channels | 1 |
| bit rate | `128,000` bit/s |

这只证明开发工具可以读取该文件的技术元数据，不证明未来 Rust importer、Symphonia、mock E2E 或真实 Provider 已通过。未计算/输出真实资产 hash，未复制文件，未读取标签，未转写内容。

当前开发机存在 `ffmpeg`、`ffprobe`、`ffplay` 8.0.1；它们只能作为开发验证工具。仓库仍没有 `Cargo.toml`、`package.json`、锁文件或 Git metadata，因此 Phase 0 无法运行应用类型检查、单元/集成测试或构建。

## 10. Phase 1 离线导入 PoC

### 10.1 PoC 接口

PoC 只放在 Agent 3 目录或独立 test binary，不修改生产 Provider 字段。建议验证入口：

```text
audio_import_probe inspect-and-stage
  --source <path>
  --staging-root <test-managed-root>
  --local-max-bytes <u64>
  --capabilities-profile <mock-profile>
  --json

audio_import_probe batch
  --source <path> ...
  --staging-root <test-managed-root>
  --capabilities-profile <mock-profile>
  --json
```

stdout 只返回 artifact ID、container/codec/MIME、byte length、duration、status 和安全错误码；不能返回源/受管路径、文件名、hash 或 parser dump。所有 Rust 函数添加函数级注释。

### 10.2 fixture 生成与基础命令

Phase 1 在隔离 `.artifacts` 子目录生成非敏感 997 Hz 短音频；这些工具不进入应用运行依赖：

```powershell
$ErrorActionPreference = 'Stop'
$CaseRoot = Join-Path (Join-Path $PWD '.artifacts\audio-import-poc') ([guid]::NewGuid().ToString('N'))
$SourceRoot = Join-Path $CaseRoot 'source'
$StagingRoot = Join-Path $CaseRoot 'managed-staging'
New-Item -ItemType Directory -Force -Path $SourceRoot, $StagingRoot | Out-Null

$ValidWav = Join-Path $SourceRoot 'valid.wav'
$ValidMp3 = Join-Path $SourceRoot 'valid.mp3'
$ValidM4a = Join-Path $SourceRoot 'valid.m4a'
ffmpeg -hide_banner -loglevel error -y -f lavfi `
  -i 'sine=frequency=997:sample_rate=16000:duration=2' -c:a pcm_s16le $ValidWav
ffmpeg -hide_banner -loglevel error -y -f lavfi `
  -i 'sine=frequency=997:sample_rate=16000:duration=2' -c:a libmp3lame -b:a 64k $ValidMp3
ffmpeg -hide_banner -loglevel error -y -f lavfi `
  -i 'sine=frequency=997:sample_rate=16000:duration=2' -c:a aac -b:a 64k $ValidM4a

$EmptyMp3 = Join-Path $SourceRoot 'empty.mp3'
New-Item -ItemType File -Path $EmptyMp3 | Out-Null

$TruncatedMp3 = Join-Path $SourceRoot 'truncated.mp3'
Copy-Item -LiteralPath $ValidMp3 -Destination $TruncatedMp3
$TruncatedStream = [System.IO.File]::Open(
  $TruncatedMp3,
  [System.IO.FileMode]::Open,
  [System.IO.FileAccess]::Write,
  [System.IO.FileShare]::None
)
$TruncatedStream.SetLength(64)
$TruncatedStream.Dispose()

$WrongExtension = Join-Path $SourceRoot 'wav-renamed-as.mp3'
Copy-Item -LiteralPath $ValidWav -Destination $WrongExtension

$OversizeMp3 = Join-Path $SourceRoot 'oversize.mp3'
$OversizeStream = [System.IO.File]::Open(
  $OversizeMp3,
  [System.IO.FileMode]::CreateNew,
  [System.IO.FileAccess]::Write,
  [System.IO.FileShare]::None
)
$OversizeStream.SetLength(1048577)
$OversizeStream.Dispose()

cargo check --manifest-path .\src-tauri\Cargo.toml
cargo test --manifest-path .\src-tauri\Cargo.toml offline_import
```

生成的 `oversize.mp3` 是 sparse/无效测试对象；在注入 `1,048,576` byte 上限时必须先返回 `file_too_large`，证明超限检查发生在 parser 和 staging copy 之前。

### 10.3 源文件只读与 staging 验收脚本

```powershell
$ReadOnlySource = Join-Path $SourceRoot 'read-only-source.wav'
Copy-Item -LiteralPath $ValidWav -Destination $ReadOnlySource
& attrib.exe +R $ReadOnlySource
$BeforeItem = Get-Item -LiteralPath $ReadOnlySource
$BeforeLength = $BeforeItem.Length
$BeforeWriteTime = $BeforeItem.LastWriteTimeUtc
$BeforeHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ReadOnlySource).Hash

$ImportJson = & cargo run --quiet --manifest-path .\src-tauri\Cargo.toml `
  --bin audio_import_probe -- inspect-and-stage `
  --source $ReadOnlySource `
  --staging-root $StagingRoot `
  --local-max-bytes 1048576 `
  --capabilities-profile mock-wav-mp3-m4a `
  --json
if ($LASTEXITCODE -ne 0) { throw 'read-only source import failed' }
$ImportResult = $ImportJson | ConvertFrom-Json
if ($ImportResult.status -notin @('Ready', 'Duplicate')) { throw 'unexpected import status' }

$AfterItem = Get-Item -LiteralPath $ReadOnlySource
$AfterHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ReadOnlySource).Hash
if ($AfterItem.Length -ne $BeforeLength) { throw 'source length changed' }
if ($AfterItem.LastWriteTimeUtc -ne $BeforeWriteTime) { throw 'source write time changed' }
if ($AfterHash -ne $BeforeHash) { throw 'source bytes changed' }
if (-not $AfterItem.IsReadOnly) { throw 'source ReadOnly attribute changed' }

$PartFiles = @(Get-ChildItem -LiteralPath $StagingRoot -File -Filter '*.part' -Recurse)
if ($PartFiles.Count -ne 0) { throw 'completed import left .part files' }
$LeakedNames = @(Get-ChildItem -LiteralPath $StagingRoot -File -Recurse |
  Where-Object { $_.Name -like '*read-only-source*' })
if ($LeakedNames.Count -ne 0) { throw 'managed filename leaked source display name' }
```

测试完成后只删除经解析确认位于 `.artifacts\audio-import-poc\<case-id>` 下的 `$CaseRoot`；不得使用未验证变量或宽泛 glob 清理。先恢复生成 fixture 的 ReadOnly 属性。真实仓库 MP3 不由清理脚本删除。

### 10.4 现有真实 MP3 + mock capability

```powershell
$env:TEST_AUDIO_PATH = (Resolve-Path -LiteralPath 'AI视频批量生产与模板优化会议.mp3').Path
$RealAssetJson = & cargo run --quiet --manifest-path .\src-tauri\Cargo.toml `
  --bin audio_import_probe -- inspect-and-stage `
  --source $env:TEST_AUDIO_PATH `
  --staging-root $StagingRoot `
  --local-max-bytes 67108864 `
  --capabilities-profile mock-wav-mp3-m4a `
  --json
if ($LASTEXITCODE -ne 0) { throw 'real MP3 import failed' }
$RealAssetResult = $RealAssetJson | ConvertFrom-Json
if ($RealAssetResult.status -notin @('Ready', 'Duplicate')) { throw 'real MP3 was not accepted' }
```

PoC stdout、普通日志、测试报告和 snapshot 不得包含环境变量值、真实文件名/路径、hash、标签或正文。测试后按受管 artifact 清理协议删除 staging 副本，确认根目录源文件仍存在且 metadata 未变化。

### 10.5 精确验收标准

| ID | Go 判据 | Fail / BLOCKED 判据 |
| --- | --- | --- |
| POC-IMP-01 | Windows 单选返回 0/1 项，多选返回 0..N 项；取消为零结果 | 取消被当错误、前端获得通用文件系统权限 |
| POC-IMP-02 | 生成的 WAV/MP3/M4A 均识别出正确 container、codec、MIME、2 秒左右时长 | 只依赖扩展名、格式误判、无音轨仍 ready |
| POC-IMP-03 | 0 字节返回 `empty_audio`，Provider 调用 0 次，无 staging 完成文件 | 上传、panic、ready |
| POC-IMP-04 | 头/尾截断 fixture 返回 `corrupt_audio`，Provider 调用 0 次 | 仅探测头部后 ready |
| POC-IMP-05 | WAV 改名 `.mp3` 返回 `extension_content_mismatch` | 当作 MP3 或静默改名 |
| POC-IMP-06 | 注入 1 MiB 上限时 1 MiB + 1 byte 文件返回 `file_too_large`，不创建 staging | parser 先运行、整数溢出、部分副本残留 |
| POC-IMP-07 | mock capability 分别验证 MIME/byte/duration 超限；每种失败网络调用 0 次 | capability 缺失被当作已验证支持 |
| POC-IMP-08 | ReadOnly 源导入成功且 size/hash/mtime/属性不变；删除 artifact 后源仍存在 | 任一源 metadata/bytes 被修改或删除 |
| POC-IMP-09 | copy 后 source 被改变的注入测试返回 `source_changed_during_import`，`.part` 清理 | 上传混合/变化中的 bytes、ready |
| POC-IMP-10 | 同 bytes 不同文件名并发导入只产生一个 managed artifact；结果引用同 artifact ID | 两份副本、按路径错误去重、竞态失败 |
| POC-IMP-11 | 同 artifact 使用不同模板可创建不同 task；同配置活动任务仍幂等 | artifact 去重误删合法业务任务 |
| POC-IMP-12 | 混合批次 valid WAV + corrupt MP3 + valid M4A 返回 Ready/Failed/Ready | 单项失败终止批次或批次显示全成功 |
| POC-IMP-13 | 复制、哈希、完整解码均流式、有界、可取消；取消后无完成 artifact | `read_to_end`、无界队列、取消后继续处理 |
| POC-IMP-14 | 现有 31,185,261-byte MP3 在 mock capability 下 ready，duration 与 ffprobe 差异不超过 1 秒 | 未运行却声称通过、日志暴露路径/hash/内容 |
| POC-IMP-15 | staging 最终文件名为 opaque ID，IPC 无 source/managed path，完成后无 `.part` | 文件名/路径泄漏、宽泛清理 |
| POC-IMP-16 | 无 ffmpeg PATH 的干净 Windows 环境仍可导入三种已支持格式 | 客户端暗中依赖开发机 FFmpeg |

`POC-IMP-09`、不可读文件和 disk-full 使用注入 reader/writer/file-metadata adapter 的确定性测试，不通过修改系统目录 ACL 或填满系统盘制造条件。

## 11. 实施结果、未验证事项与风险

- Rust importer 已完成编译；WAV/MP3/M4A fixture 与仓库真实 MP3 均已通过自动测试。
- AAC-LC、ALAC、ID3、异常 ISO/MP4、超长文件和多音轨兼容性尚未实测。
- 完整解码 30 分钟、数小时和接近本地上限文件的 CPU、内存、取消延迟尚未测量。
- Windows ReadOnly、文件锁、UNC、OneDrive 占位、移动盘断开、源文件并发变化和磁盘满尚未实测。
- staging 的显式释放、终态清理、启动清理和去重已实现；app-local-data 实际 ACL、锁定文件的持久化 `cleanup_pending`、保留期限及卸载残留尚未实测。
- 真实 Provider 的 accepted MIME、byte/duration limit、上传方式和是否需要转码均未知；只能由 Phase 4 证据更新。
- 应用骨架和 Git metadata 已建立；类型检查、单元/集成测试、前端构建、Tauri dev 和 NSIS 构建均已执行。

通过文件导入与 Mock 全流程不等于真实 ASR 已识别音频。未执行的 Windows 边界条件和真实 Provider 验证继续按 `docs/test-plan.md` 标记为 NOT RUN 或 BLOCKED。
