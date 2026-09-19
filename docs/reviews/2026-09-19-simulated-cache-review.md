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

## 修复后的行为

本次修复把模拟缓存定义为“每个供应商尝试选择一次命中比例，再把该比例应用到输入 token”。有上游真实 usage 时使用真实总输入；缺少 usage 时，仅内部统计回退到请求估算。它不缓存模型响应，也不保证上游真实成本降低。

模块开关与 provider 开关同时开启才生效。通用百分比配置优先于旧 Kiro 配置，旧 Kiro 前缀缓存逻辑保留。启用百分比模拟会覆盖上游原有缓存读写分类。

| 客户端接口 | 同步 / HTTP 流式 / WebSocket 覆盖 | 写回字段 |
| --- | --- | --- |
| OpenAI Chat Completions | 同步、SSE | `usage.prompt_tokens_details.cached_tokens` |
| OpenAI Responses / compact | 同步、SSE；Responses WebSocket | `usage.input_tokens_details.cached_tokens`；SSE/WS 使用 `response.usage` |
| Claude Messages / Claude CLI | 同步、SSE，包含跨格式转换后的响应 | `usage.cache_read_input_tokens`；`input_tokens` 保持 fresh input，缓存创建量归零 |
| Gemini generateContent / Gemini CLI | 同步、SSE，包含跨格式转换后的响应 | `usageMetadata.cachedContentTokenCount`；`promptTokenCount` 保持总输入 |
| Gemini Interactions | 同步、SSE | `usage.total_cached_tokens`；流式为 `interaction.usage` |
| 图片、视频、音频生成、embedding 等独立接口 | 排除 | 不注入模拟缓存 |

文本接口中明确要求媒体输出的请求也排除：Responses 强制 `image_generation`，Chat 音频输出，Gemini/Interactions 图片、音频或视频输出 modalities。输入图片、音频让模型理解并返回文本，仍可使用模拟缓存。仅声明一个可选的图片生成工具不等于此次请求已在生成图片。

客户端只改写上游已经提供的 usage；不会凭空新增缺失的 usage 或把错误终态包装为成功。Responses 支持携带 usage 的 `response.completed`、`response.done`、`response.incomplete`。SSE 支持跨字节块、CRLF、多行 data 记录；未知 JSON 字段保留。

## 审查问题与对应修复

| 原问题 | 修复与回归覆盖 |
| --- | --- |
| P1：同步请求把已包含缓存的总输入再次加上缓存量 | `aether-usage/runtime/src/write.rs` 使用真实 gross input 计算百分比，不再重复加缓存。回归覆盖真实输入 1000、50% 命中，结果 input=1000/read=500；另覆盖 estimate 大于实际、真实零输入、Claude/Gemini。 |
| P2：Claude/Gemini 同步和 SSE 没有模拟字段 | `aether-ai/formats/src/formats/shared/simulated_cache.rs` 提供协议共用计算与原生 usage 映射；在目标协议转换完成后重写。Claude SSE 从 message_start 保存输入，用于只有 output 的 message_delta。 |
| P2：Responses WebSocket 完全绕过模拟缓存 | 每个 attempt 读取配置并固定比例，observer 先处理真实 usage，再改写客户端副本；PII 还原继续在最后执行。终态统计复用相同比例。新增真实 gateway/mock upstream/PostgreSQL E2E，覆盖两轮 continuation、客户端返回和计费记录。 |
| P2：非文本接口没有统一排除 | gateway 的 `execution_runtime/simulated_cache.rs` 按文本格式白名单、图片 plan 标记、明确输出类型筛选。回归覆盖媒体输出排除、图片/音频输入仍允许。 |
| P2：估算缓存量超过真实输入，或者同步/流式重新随机 | report context 保存单次选择的 basis points；真实 usage 出现后重算。read 不超过 gross input，0%/100% 均有回归。 |
| Interactions 原生 usage 未完整进入内部统计 | usage mapper 和流式轻量 envelope 增加 `interaction.usage` 与 `total_*` 字段，覆盖同步和终态 SSE 提取。 |

Interactions 字段依据已核实的 Google 官方 SDK：
https://github.com/googleapis/python-genai/blob/main/google/genai/_gaos/types/interactions/usage.py

## 验证与部署边界

- 按用户要求，修复阶段不在本地运行构建、测试或安装依赖；只进行代码静态审查、Git diff 检查与 rustfmt 格式化。
- Rust CI 在 main push 自动执行格式、Clippy、单元和集成测试。新增 `Frontend CI`，执行依赖安装、只读 ESLint、类型检查、Vitest 与标准生产构建，包含 VSCodex Web prebuild 依赖。
- 上游合并阶段适配了 ProviderFormDialog 的模块 store fixture，并纠正旧 Chat SSE 测试误声明 Responses provider 格式的问题。本轮修复以 GitHub Linux CI 结果为准。
- 当前修复的远端 CI 结果待推送后补充；没有执行真实供应商调用、服务器更新、容器部署或发布 tag。
