# Command Code 提供商

Command Code 原生适配通过 HTTP 调用 `https://api.commandcode.ai/alpha/generate`，不需要启动 Command Code CLI 或额外的代理服务。

## 配置

1. 在提供商页面新增 **Command Code**。系统创建固定的 `openai:chat` 端点并默认启用格式转换。
2. 添加 Key，填写完整的 `user_*` 上游凭据，不添加 `Bearer ` 前缀。凭据按现有加密机制保存；不支持自动登录或刷新。
3. 获取模型列表，或者手动添加上游模型名并配置全局模型映射。模型查询使用 `/provider/v1/models`；查询失败不代表静态列表中的模型有权限，需按账号实际权限配置。
4. 客户端使用 Aether 自己的 API Key，访问 `/v1/chat/completions`、`/v1/responses` 或 `/v1/messages`。

## 支持范围

- 支持文本、system/developer 指令、函数工具声明、工具调用和工具结果、思考文本及流式/非流式输出。
- 上游始终流式，`stream:false` 由 Aether 聚合后返回 JSON。
- 支持 `max_tokens`/`max_completion_tokens`（1–200000，默认 64000）、temperature、low/medium/high reasoning effort、tool choice 和 parallel tool calls；跨格式请求仍受 Aether 格式转换契约约束。
- 首版拒绝图片、文件、音频、结构化输出、strict function schema、未映射的采样参数及平台内置工具。`store:true` 和 `previous_response_id` 不支持，每轮需要提交完整必要历史。
- 工具由客户端执行；保留原始工具名称，不做会造成名称冲突的 CLI 内置工具别名替换。思考内容不代表 Anthropic 官方可验证签名。
- 必须收到有效 `finish` 才会输出成功结束。`finish-step` 或单纯 EOF 不视为完成；坏 JSON、过大 NDJSON 行和中途错误会终止请求。

## 初始化、会话与用量

生成前进行指纹和生命周期初始化。身份按凭据确定性派生，不读取网关宿主机身份或真实工作目录。初始化按凭据、地址、代理和传输配置单飞；最多等待同组初始化 6 秒，初始化网络请求合计最多 5 秒；失败仍尝试生成。缓存限额为 4096 个组合，成功 8–10 小时刷新，失败 1–5 分钟后允许重试。初始化复用生成请求的代理、TLS 和 HTTP 设置。

生成会话来自 `x-session-id`、`x-claude-code-session-id`、`session_id` 请求头或 `prompt_cache_key` 请求字段，并与网关已认证的下游 Key ID、上游 Key ID 共同派生 UUID，防止跨调用方复用。无显式会话或缺少调用方身份时，每次请求生成独立 UUID。会话标识不提供服务端对话存储。

缓存读写作为总输入 token 的组成部分记录，三种协议按各自用量口径输出；没有完整上游 usage 时，交给 Aether 现有缺失用量策略处理。`pause_turn` 映射为长度限制/未完成状态。无法识别的上游事件会明确失败，便于发现协议变化。

## 验证边界

协议参考 CommandCodeGo-manager `bdbd68ef47d42e7ffdb9a7278c9eb219f42f0ff7`（v0.2.10），CLI 协议版本为 `1.53.1`。模拟测试不能证明当前账号的在线模型权限、初始化接口要求或上游协议未变更。上线前需使用有效凭据验证三个端点的文本、工具及流式/非流式响应。

Rust 校验遵循本仓库约定：本地只做 rustfmt 和 Cargo metadata；编译、Clippy 和 Rust 测试由 GitHub Actions 执行。
