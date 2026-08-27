# 微信公众号读书笔记 MCP 设计

## 目标

把结构化读书笔记稳定渲染为统一的公众号文章，通过可部署到云服务器的 MCP 管理草稿与发布流程，同时不向模型、日志或磁盘暴露 Access Token。

首版围绕单篇普通图文 `news`，不提供任意微信 API 代理。Markdown、任意富文本、正文图片、多图文组合、图片消息、商品、微信读书同步和公网回调均不在范围内。

## 架构

```text
MCP 客户端
    │ Streamable HTTP /mcp + we-user Header
    ▼
Axum HTTP ── Host 校验 / 请求上限 / Header 校验
    │
    ▼
WechatMpMcp ── 参数校验 / 写开关 / 二次确认
    ├── BookNoteRenderer ── 强类型 JSON → Handlebars → 转义后的 HTML
    └── WechatClient
          ├── Stable Token 内存缓存
          ├── JSON / multipart 请求
          ├── 响应大小与 errcode 转换
          └── WECHAT_MEDIA_ROOT 文件边界
                    │
                    ▼
              微信公众号 API
```

HTTP 是默认传输，stdio 仅作为本机兼容模式。`/mcp` 要求 Header `we-user: tobeurself`，`/healthz` 仅返回存活状态。初始化错误和运行时日志只能写 stderr，且不得包含 AppSecret、Access Token 或带 Token 的完整 URL。

对于非回环监听，服务要求显式 Host allowlist；公网部署应再由 HTTPS 反向代理、云安全组或私网保护。Header 校验是轻量访问控制，不替代网络层防护。

## 渲染模型

模板使用项目内固定的 `templates/book_note.html.hbs`。Handlebars 开启严格模式，缺失变量直接失败。

外部输入不直接作为可信 HTML：普通标题交给 Handlebars 默认转义；允许换行的字段先执行 `& < > " '` 转义，再把换行转换成 `<br>`，最后由模板中的内部 triple-stash 插入。只有服务端生成的已转义片段会进入 triple-stash。

模板正文不重复微信文章标题，而显示 `22px` 的《书名》小标题。核心观点按输入顺序编号，行动项使用固定勾选样式。

## Token 与请求

- 使用 `POST /cgi-bin/stable_token`，固定 `force_refresh=false`；
- Token 只缓存在进程内，在 `expires_in` 前五分钟失效；
- Mutex 覆盖 Token 获取，避免并发冷启动重复请求；
- API 返回 `40014` 或 `42001` 时清缓存并重放一次已被微信拒绝的请求；
- 不自动使用强制刷新，避免每日次数限制和多实例互相使 Token 失效；
- 不自动重试其他写请求，避免网络结果不明确时重复创建或发布；
- HTTP 非 2xx 和 HTTP 200 中的非零 `errcode` 都转为 MCP tool error。

## 文件与写操作安全

`WECHAT_MEDIA_ROOT` 未配置时文件上传不可用。输入路径在云服务器上先与根目录组合并 canonicalize，最终路径必须仍位于 canonical 根目录内，因此 `..` 和指向目录外的符号链接都会被拒绝。

`WECHAT_ALLOW_WRITE` 是唯一服务端写闸门，默认关闭。开启后同时允许上传、草稿写入、发布和删除。发布及删除还要求工具参数 `confirm=true`；这是防误调用信号，不替代 MCP 客户端向用户展示参数和确认意图。

封面路径模式是“上传永久素材 → 创建/更新草稿”的两阶段操作，微信不提供事务。若第二步失败，错误必须携带 `uploaded_cover_media_id`，让调用方复用已经产生的素材。

已发布文章删除用显式 `scope` 消除微信 API 的危险默认值：`single` 必须有正整数 `index`，`all` 必须没有 `index`，再映射为微信的 `index=0`。

## 发布状态

`publish_draft` 只返回提交结果和 `publish_id`。MCP 不启动公网 callback server；调用方使用 `get_publish_status` 查询状态：成功、发布中、原创失败、常规失败、平台审核失败、发布后全删、系统封禁。

## 测试边界

单元和 Mock 测试覆盖模板转义、输入约束、文件边界、Token 缓存、微信错误映射、工具路由、写开关和删除语义。真实账号默认只允许 Token 与草稿数量的只读冒烟测试；真实上传、草稿、发布和删除必须由用户单独授权。
