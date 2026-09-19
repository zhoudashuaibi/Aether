# 上游同步与模拟缓存审查

审查日期：2026-09-19。模型：GPT-6 Astra。

## 同步范围

- fork 同步前：`92a4597ea545e3f270bbaea6f16ffb644c860569`。
- 本次上游：`fawney19/Aether` 的 `main`，`ba7c9f8b270cce63b0515299076b30129d7d64b4`。
- 上游新增 245 个提交，合并前差异涉及 1,781 个文件。
- 保留 fork 的模拟缓存配置、管理界面、统计与 OpenAI 响应写回；保留 `ghcr.io/zhoudashuaibi/aether`、`GHCR_TOKEN`、Linux amd64/arm64 发布。
- 上游新增 Nightly 使用 fork 镜像地址，并沿用 fork 的 GHCR 凭据；安装说明中的镜像和安装脚本指向 fork。
- 同名标签 `v0.1.0`、`v0.1.1`、`v0.1.2` 未覆盖。后续单独拉取 `upstream/main` 成功。
- 上游已正式移除 SQLite/MySQL，仅支持 PostgreSQL。原 SQLite/MySQL 部署不能直接用这次版本替换，需要另行迁移数据。此次没有操作服务器、数据库或容器。

七个冲突文件的处理：

| 文件 | 原因及处理 |
| --- | --- |
| `.github/workflows/release.yml` | 合并上游固定版本 action、provenance 和 Linux 打包逻辑，保留 fork GHCR 配置，移除 fork 未使用的 Docker Hub 登录和 provenance。 |
| `README.md`、`install.sh` | 保留 fork 下载地址，接入新的 compose 安装参数与 nightly 说明。 |
| `execution_runtime/kiro_web_search.rs` | 保留通用模拟缓存结算，避免恢复旧 Kiro 重复计量；采用上游新的失败响应脱敏与受限解析。 |
| `execution_runtime/stream/execution.rs` | 保留通用模拟缓存配置读取、计量和 Chat 流配置预置，接入上游新的 frame codec、取消和流式执行逻辑。 |
| `execution_runtime/sync/execution.rs` | 同时保留 fork 缓存写回和上游 candidate skip 逻辑。 |
| `ProviderFormDialog.vue` | 保留模拟缓存范围校验；不恢复上游已移除的旧月卡表单字段。 |

同步时还适配了上游 ProviderFormDialog 测试对模块 store 的依赖；纠正旧 Chat SSE 测试把 Chat 字节流声明为 Responses 上游的 fixture。下面的运行时缺陷尚未修复。

## 审查结论

模拟缓存没有完整覆盖所有文本接口。当前功能根据 provider 配置，把估算输入的一段随机比例标为缓存命中；它没有缓存模型响应，也不保证上游真实成本降低。

“内部统计已接入”仅说明执行链会注入模拟 usage，不代表计费正确或客户端能看到相同字段。

| 接口 | 内部模拟统计 | 客户端同步响应 | 客户端流式响应 |
| --- | --- | --- | --- |
| OpenAI Chat Completions | 已接入，有下面的计量问题 | 写入 `usage.prompt_tokens_details.cached_tokens` | 上游返回含 `prompt_tokens` 的 usage 时写入；不会凭空生成缺失的 usage |
| OpenAI Responses / compact，HTTP | 已接入，有下面的计量问题 | 写入 `usage.input_tokens_details.cached_tokens` | 只处理含 `response.usage.input_tokens` 的 `response.completed`；其他终态没有该补写 |
| Claude Messages / 基于 Messages 的客户端 | 已接入 | 未补写模拟缓存 | 未补写模拟缓存 |
| Gemini generateContent / 基于它的客户端 | 已接入 | 未补写模拟缓存 | 未补写模拟缓存 |
| Gemini Interactions | 通用执行路径已接入 | 未补写模拟缓存 | 未补写模拟缓存；需进一步确定该协议的原生缓存字段契约 |
| OpenAI Responses WebSocket | 未接入 | 不适用 | 未接入，只观察和传递真实上游 usage |
| 图片、视频等非文本任务 | 缺少统一排除，走通用执行链的请求可能被模拟计量 | 不应启用 | 不应启用 |

跨格式请求按**客户端目标格式**决定最后的补写：目标为 OpenAI 才有通用模拟字段补写；目标为 Claude/Gemini 的真实上游缓存字段可以经转换传播，但模拟值没有接入该转换链。

## 确定问题

### P1：同步请求将已包含缓存的总输入再次加上缓存 token

- 位置：`crates/aether-usage/runtime/src/write.rs:1113-1125`。
- 前置：`simulated_cache_standardized_usage_from_context` 在同文件 `2051-2059` 对 OpenAI/Gemini 返回 gross input，即已经包含缓存的输入总量。
- 合并函数却无条件执行 `input + cache_creation + cache_read`，再和真实上游 usage 取最大值。
- 具体例子：上游输入 1,000，上下文输入 1,000，模拟命中 500。同步记录变成输入 1,500、缓存 500；流式记录则为输入 1,000、缓存 500。
- `crates/aether-billing/src/token_normalization.rs:40-48` 再扣除缓存后，同步按 1,000 个普通输入 token 计费，流式按 500 个计费；同步还会另外计入缓存读费用。
- 修复方向：统一内部 usage 的 gross/fresh 契约，只有 fresh input 才应加回缓存重建总量；增加同步与流式一致性测试。现有测试使用上游 2,200、上下文 1,800、缓存 300，`max` 掩盖了重复相加问题（`write.rs:6475`）。

### P2：Claude/Gemini 的客户端响应没有模拟缓存

- 位置：`crates/aether-ai/formats/src/formats/openai/simulated_cache.rs:38-46`。
- 触发：模块与 provider 均启用模拟缓存，客户端使用 Claude/Gemini；上游本身没有缓存命中字段。
- 同步执行已把模拟 token 放进 context，但 `sync_finalize.rs:71-94` 和 `sync/execution.rs:3262-3267` 只调用 OpenAI 补写器。
- SSE 的 `finalize/internal/stream_rewrite.rs:20-25` 也只实例化同一个 OpenAI 模拟缓存补写器；普通格式转换读取的是真实上游 usage，不会读取这组模拟 context。
- 影响：Aether 内部日志/计费有模拟命中，Claude 的 `usage.cache_read_input_tokens`、Gemini 的 `usageMetadata.cachedContentTokenCount` 没有对应模拟值。跨格式输出到这两类协议也存在相同问题。
- 修复方向：按最终客户端协议写回同步/SSE usage，并按各协议的 gross/fresh 语义保持总量一致。

### P2：Responses WebSocket 绕过整个模拟缓存流程

- 位置：`apps/aether-gateway/src/handlers/proxy/websocket/responses/turn.rs:1171-1195`、`:1228-1309`。
- 触发：对启用了模拟缓存的 provider 使用 Responses WebSocket。
- 该路径独立构造 report context、观察真实上游事件并完成 usage；没有 HTTP 执行链的配置读取、模拟计算和补写器。
- 搜索范围：`handlers/proxy/websocket`、`ai_serving/planner`、`formats/shared`；搜索式 `simulated_cache` 未命中。结合 WS 终态与 context 构造调用链，可确认当前 WS 没有接入该功能。
- 影响：同一 provider 的 HTTP 与 WebSocket 缓存统计/客户端字段不一致。
- 修复方向：在每个 WebSocket turn/attempt 的生命周期接入相同计量策略，并覆盖重试、取消及复用连接，避免重复计费。

### P2：非文本接口没有统一排除

- 位置：`apps/aether-gateway/src/execution_runtime/sync/execution.rs:2882-2895`；流式对应 `stream/execution.rs:2944-2956`、`:5983-5995`。
- 触发：一个启用模拟缓存的 provider 同时服务文本和图片等接口，非文本请求进入通用成功处理。
- 配置读取和 seed 只检查成功状态及 provider 配置，没有按文本 plan/任务类型筛选。图片与视频 planner 也写入 `original_request_body`；估算器在 `kiro_cache.rs:704-730` 会统计 `prompt`/`input`，甚至退回整个 JSON 估算。
- usage 写入在 `write.rs:2033-2060` 仅检查模拟开关，不检查任务类型。
- 影响：图片等请求可产生虚构的输入/缓存统计。最终是否改变金额取决于模型价格规则，不能断言所有图片/视频都按 token 多收费。Embedding/rerank 也不在明确白名单之外，应单独决定产品策略。
- 修复方向：使用明确的文本生成能力/任务白名单，同时识别走 Chat/Responses 格式的图片生成任务，不能只检查 URL 或格式前缀。

### P2：模拟值按预估 token 生成，未限制在真实响应的输入总量内

- 位置：`apps/aether-gateway/src/execution_runtime/stream/execution.rs:904-922`；`crates/aether-ai/formats/src/formats/openai/simulated_cache.rs:152-172`。
- 触发：token 估算高于上游真实 usage，例如估算 1,000、实际上游输入 300、配置 100% 命中。
- 流式统计使用 `max(actual, estimate)`，缓存重写器又直接写入预先算好的 1,000，不以响应中的 300 为上界。结果可能是客户端 `prompt_tokens=300`、`cached_tokens=1000`，内部输入却是 1,000。
- 影响：缓存子集大于输入总量，客户端与内部统计不一致；配置的命中百分比也不再对应实际 token。
- 修复方向：以真实上游 usage 为准，仅在缺失时估算；每次请求只选一次比例，再以权威输入总量计算和限幅。同步、SSE 与内部持久化共用结果。

“开启模拟后替换真实缓存”可能是该功能的产品意图，本次不单独把它判为缺陷；但是否保留真实缓存、仅补零缓存，建议后续明确配置语义。

## 验证边界

- 本次是源码调用链审查，没有连接真实 Claude/OpenAI/Gemini 服务，也没有修改运行中的服务器。
- 按项目约束不在本地编译 Rust；`cargo fmt --all --check`、`cargo metadata --locked --offline --no-deps --format-version 1` 和 Git diff 检查通过。合并后的 Rust 构建/测试尚需 GitHub Actions 验证。
- 前端类型检查通过；模拟缓存相关提供商摘要/表单测试 14 项通过。全量首次运行 214 个文件通过；9 项表单测试的模块 store fixture 已适配并定向复测通过。另有 3 个认证组件 suite 在 Windows 因 `file:///aether_adaptive.svg` 加载失败。
- 标准前端构建的 prebuild 在 Windows 报 `spawnSync npm.cmd EINVAL`；VSCodex Web 独立构建通过，随后手动复制其产物并执行 `npm run --ignore-scripts build:with-typecheck`，主前端生产构建通过。标准 prebuild 的 Windows 兼容问题仍存在。
- 发布供应链测试在仅转换行尾的临时快照上通过，覆盖固定 action/base image、provenance、Linux 双架构；`install.sh` shell 语法检查通过。
- Compose 配置测试在当前 Windows 的 Python 子进程中报 Docker CLI `unknown flag: --project-name`，未算通过；没有启动容器。
- 本次没有推送、打标签或发布镜像。发现 P1 计费问题，应先修复并在 CI 验证后再推送正式分支/发布。
