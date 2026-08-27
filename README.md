# 微信公众号读书笔记 MCP

可部署到云服务器的 Streamable HTTP MCP Server：把结构化读书笔记 JSON 渲染为固定的微信公众号 HTML 模板，并管理封面、草稿、发布任务和已发布文章。HTTP MCP 端点是 `/mcp`，同时保留可选 stdio 模式。

设计与安全边界见 [DESIGN.md](./DESIGN.md)。模板位于 [templates](./templates/)：`reading`、`business`、`tech`、`invest` 四种 `style` 会分别选择对应模板。

## 能力

- 将书名、作者、阅读原因、总结、核心观点、案例、思考、读者和行动项渲染成通用阅读模板；
- 所有笔记字段按纯文本处理，自动 HTML 转义，换行转换为 `<br>`；
- 从受限本地目录上传永久封面，或复用已有永久素材 `media_id`；
- 查询公众号永久图片素材，获取可复用的 `media_id`；
- 创建、查询、列表、更新和删除草稿；
- 提交异步发布、查询状态、读取列表及删除已发布文章；
- Stable Token 内存缓存、提前刷新和失效重试；
- 默认禁止所有写操作。

当前不支持 Markdown、字段内自定义 HTML、正文图片、多图文组合、图片消息、商品信息、微信读书同步或发布回调服务。

## 构建

```bash
cd /path/to/wechat-mp-mcp
sh scripts/build-release.sh
```

构建产物位于 `dist/wechat-mp-mcp`。普通开发验证可运行：

```bash
cargo test
```

## 配置

| 环境变量 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `WECHAT_APP_ID` | 是 | 无 | 公众号 AppID |
| `WECHAT_APP_SECRET` | 是 | 无 | 公众号 AppSecret，不写入日志或工具返回 |
| `WECHAT_ALLOW_WRITE` | 否 | `false` | `1`、`true` 或 `yes` 时允许素材上传、草稿写入、发布和删除 |
| `WECHAT_MEDIA_ROOT` | 文件上传时 | 无 | MCP 可读取媒体文件的唯一根目录 |
| `WECHAT_MAX_RESPONSE_BYTES` | 否 | `1048576` | 单次微信 API 响应最大字节数 |
| `WECHAT_TRANSPORT` | 否 | `http` | `http` 使用 Streamable HTTP；`stdio` 使用本机标准输入输出 |
| `WECHAT_HTTP_BIND` | HTTP | `127.0.0.1:8000` | HTTP 监听地址；云服务器可设为 `0.0.0.0:8000` |
| `WECHAT_HTTP_ALLOWED_HOSTS` | 非回环 HTTP | 无 | 逗号分隔的 Host allowlist，例如 `203.0.113.10:8000,mcp.example.com` |
| `WECHAT_HTTP_ALLOWED_ORIGINS` | 否 | 无 | 逗号分隔的浏览器 Origin；普通 MCP 客户端通常不发送 Origin |
| `WECHAT_HTTP_MAX_REQUEST_BYTES` | 否 | `1048576` | 单次 MCP POST 最大请求体 |

云服务器环境变量示例：

```bash
WECHAT_APP_ID=your-app-id
WECHAT_APP_SECRET=your-app-secret
WECHAT_ALLOW_WRITE=false
WECHAT_MEDIA_ROOT=/var/lib/wechat-mp-mcp/media
WECHAT_TRANSPORT=http
WECHAT_HTTP_BIND=0.0.0.0:8000
WECHAT_HTTP_ALLOWED_HOSTS=203.0.113.10:8000
```

远程 MCP 地址与认证 Header：

```text
http://203.0.113.10:8000/mcp
we-user: tobeurself
```

`GET /healthz` 不需要 Header，可用于存活检查。需要兼容本机 stdio 时，设置 `WECHAT_TRANSPORT=stdio`。

先用只读模式确认 `get_draft_count` 可用，再按需设置 `WECHAT_ALLOW_WRITE=true`。注意：这是单一写开关，开启后发布和不可逆删除工具也会开放；发布和删除仍要求显式传入 `confirm=true`。

公众号后台还必须把云服务器的固定出口 IP 加入 IP 白名单，否则 Stable Token 会返回 `40164 invalid ip`。

## 云服务器部署

服务默认监听 `127.0.0.1:8000`；若通过公网 IP+端口调用，设置 `WECHAT_HTTP_BIND=0.0.0.0:8000`，并把公网 IP 或域名写入 `WECHAT_HTTP_ALLOWED_HOSTS`。所有 `/mcp` 请求必须包含精确的 Header：`we-user: tobeurself`。

这个 Header 是按当前要求提供的轻量校验，并非强认证。公网部署仍应使用 HTTPS 反向代理、云安全组限制来源 IP 或私网/VPN。远程模式下 `file_path` 指的是云服务器文件系统，不是 MCP 客户端本机路径；封面应预先放入 `WECHAT_MEDIA_ROOT`，或直接复用已有的 `media_id`。可参考 [systemd 示例](./deploy/wechat-mp-mcp.service.example)，敏感变量应放在权限为 `0600` 的 `/etc/wechat-mp-mcp.env`。

### Ubuntu 24 + Docker Compose

服务器已安装 Git、Docker 和 Docker Compose 时，可直接部署：

```bash
git clone https://github.com/toBeUrself/wechat-mp-mcp.git
cd wechat-mp-mcp
cp .env.example .env
mkdir -p data/media
chmod 600 .env
```

编辑 `.env`，至少填写公众号凭证，并将 `WECHAT_HTTP_ALLOWED_HOSTS` 改成客户端实际访问时使用的 `Host`。直接通过公网 IP 和默认端口访问时，例如：

```dotenv
WECHAT_APP_ID=你的公众号AppID
WECHAT_APP_SECRET=你的公众号AppSecret
WECHAT_ALLOW_WRITE=false
WECHAT_HTTP_ALLOWED_HOSTS=203.0.113.10:8000
WECHAT_HTTP_PORT=8000
```

首次启动。Compose 默认拉取 GitHub Actions 构建的 `ghcr.io/tobeurself/wechat-mp-mcp:latest`：

```bash
docker compose pull
docker compose up -d
docker compose ps
curl --fail http://127.0.0.1:8000/healthz
```

`main` 分支每次推送都会通过 GitHub Actions 构建并发布面向 Ubuntu 24 常见 `x86_64` 主机的 `linux/amd64` 镜像，同时生成 `latest` 和 `sha-*` 标签；推送 `v1.2.3` 形式的 Git 标签时还会生成版本标签。工作流使用仓库自带的 `GITHUB_TOKEN`，不需要额外配置 Registry 密钥。

GHCR 镜像首次发布后可能默认为私有。若希望服务器免登录拉取，请在 GitHub Package 设置中将其改为 Public；保持私有时，服务器需要先使用具备 `read:packages` 权限的 Token 登录 `ghcr.io`。

健康检查返回 `ok` 后，远程 MCP 地址为 `http://服务器公网IP:8000/mcp`，请求仍需携带 `we-user: tobeurself`。确认只读工具正常后，如需上传封面、创建草稿或发布文章，把 `.env` 中的 `WECHAT_ALLOW_WRITE` 改为 `true`，然后重建容器配置：

```bash
docker compose up -d --force-recreate
```

封面文件放在服务器的 `data/media/`，MCP 工具中使用容器路径，例如 `/data/media/cover.jpg`。该目录以只读方式挂载进容器，服务只能读取并上传文件。

查看日志、更新和停止服务：

```bash
docker compose logs -f --tail=100
git pull --ff-only
docker compose pull
docker compose up -d
docker compose down
```

还需完成两项云端配置：

1. 在微信公众号后台将服务器固定出口 IP 加入 IP 白名单；
2. 在云安全组中仅向可信客户端 IP 开放 TCP 8000。若使用域名，应通过 HTTPS 反向代理暴露服务，并把域名加入 `WECHAT_HTTP_ALLOWED_HOSTS`。

## 本地启动并导出完整 HTML

最简单的方式不需要启动 MCP 服务，也不需要 `WECHAT_APP_ID` 或 `WECHAT_APP_SECRET`。把下方示例中的**仅包含 note 对象**的 JSON 保存为 `note.json` 后，直接运行：

```bash
cargo run -- render note.json > book-note.html
```

`book-note.html` 就是完整微信公众号正文 HTML，可以直接检查、保存或交给后续草稿工具使用。

仓库已内置可直接运行的示例：

```bash
cargo run -- render examples/note.json > book-note.html
```

### 通过 HTTP MCP 调试

如果需要验证 MCP HTTP 调用本身，渲染工具 `render_book_note_html` 也不会请求微信 Token 或创建草稿。先在一个终端启动服务：

```bash
cargo build
WECHAT_TRANSPORT=http WECHAT_HTTP_BIND=127.0.0.1:8000 ./target/debug/wechat-mp-mcp
```

在另一个终端，把**仅包含 note 对象**的 JSON 保存为 `note.json`，例如：

```json
{
  "book_name": "系统之美",
  "author": "德内拉·梅多斯",
  "why_read": "理解系统如何运作。",
  "summary": "系统的行为主要由内部结构决定。",
  "core_points": [{"number": "01", "title": "系统思维", "content": "不要只关注事件，要关注背后的结构。", "extension": ""}],
  "example": "书中的反馈回路案例。",
  "thoughts": "理解结构之后，解决问题的方式也会改变。",
  "target_reader": "希望改善思考方式的人。",
  "actions": [{"text": "遇到问题时先画出关键变量"}]
}
```

初始化 MCP 会话，并保留服务返回的 Session ID：

```bash
init_headers=$(mktemp)
curl -sS -D "$init_headers" -o /dev/null \
  -X POST http://127.0.0.1:8000/mcp \
  -H 'we-user: tobeurself' \
  -H 'content-type: application/json' \
  -H 'accept: application/json, text/event-stream' \
  --data '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"local-curl","version":"1"}}}'
session_id=$(awk 'tolower($1)=="mcp-session-id:" {print $2}' "$init_headers" | tr -d '\r')
```

调用渲染工具并把完整 HTML 保存到 `book-note.html`：

```bash
jq -n --slurpfile note note.json '{jsonrpc:"2.0",id:2,method:"tools/call",params:{name:"render_book_note_html",arguments:{note:$note[0]}}}' \
  | curl -sS -X POST http://127.0.0.1:8000/mcp \
      -H 'we-user: tobeurself' \
      -H 'content-type: application/json' \
      -H 'accept: application/json, text/event-stream' \
      -H 'MCP-Protocol-Version: 2025-03-26' \
      -H "Mcp-Session-Id: $session_id" \
      --data-binary @- \
  | sed -n 's/^data: //p' \
  | jq -Rr 'select(length > 0) | fromjson | .result.content[0].text | fromjson | .html' \
  > book-note.html
```

`book-note.html` 即为可以送给草稿接口的完整正文 HTML。若只想查看结果，可把最后的 `> book-note.html` 去掉。日常本地预览优先使用上面的 `cargo run -- render` 命令。

## 推荐流程

1. 调用 `render_book_note_html` 检查最终 HTML 和大小。
2. 准备封面：直接在创建工具中传云服务器本地路径，或先调用 `upload_cover_image` 获得永久 `media_id`。
3. 调用 `create_book_note_draft`，在公众号后台预览草稿。
4. 需要修改时调用 `update_book_note_draft`；省略 `cover` 会保留原封面。
5. 明确确认后调用 `publish_draft`，再用 `get_publish_status` 查询异步结果。

如果自动上传封面成功、创建或更新草稿失败，错误会包含 `uploaded_cover_media_id`。复用该 ID 重试，避免重复上传永久素材。

## 读书笔记输入

```json
{
  "note": {
    "style": "reading",
    "category": "成长",
    "tags": ["认知", "成长"],
    "book_name": "系统之美",
    "author": "德内拉·梅多斯",
    "why_read": "理解系统如何运作。",
    "summary": "系统的行为主要由内部结构决定。",
    "core_points": [{"number": "01", "title": "系统思维", "content": "不要只关注事件，要关注背后的结构。", "extension": "", "example": ""}],
    "thoughts": "理解结构之后，解决问题的方式也会改变。",
    "target_reader": "希望改善思考方式的人。",
    "actions": [{"text": "遇到问题时先画出关键变量"}, {"text": "区分增强回路和调节回路"}]
  },
  "cover": {
    "type": "file_path",
    "value": "covers/systems-thinking.jpg"
  },
  "article_options": {
    "article_author": "公众号作者名",
    "need_open_comment": false,
    "only_fans_can_comment": false
  }
}
```

`cover` 也可以复用已有永久素材：

```json
{
  "type": "media_id",
  "value": "PERMANENT_MEDIA_ID"
}
```

约束：

- `title` 可选，最多 32 个字符；省略时自动使用“书名读书笔记”；
- `core_points` 和 `actions` 至少各一项，所有必填字符串不能为空；
- `article_author` 最多 16 个字符；`digest` 最多 120 个字符；
- 未传 `digest` 时使用 `summary` 的前 120 个字符；
- 最终 HTML 少于 20,000 个字符并小于 1 MiB；
- 封面支持 bmp、gif、jpg、jpeg、png，大小为 1 字节至 10 MiB；
- 本地路径规范化后必须位于 `WECHAT_MEDIA_ROOT`，符号链接不能绕过该限制。
- HTTP 部署时 `file_path` 指云服务器文件系统，不是远程 MCP 客户端的本地文件。

## 工具

| 工具 | 类型 | 说明 |
| --- | --- | --- |
| `render_book_note_html` | 读 | 只渲染模板，返回 HTML 和大小 |
| `upload_cover_image` | 写 | 上传永久封面素材 |
| `list_permanent_media` | 读 | 查询永久素材列表，默认返回图片及 `media_id` |
| `create_book_note_draft` | 写 | 渲染并创建单篇草稿 |
| `update_book_note_draft` | 写 | 更新指定草稿文章，索引从 0 开始 |
| `get_draft` | 读 | 获取草稿详情 |
| `list_drafts` | 读 | 获取草稿列表，默认不返回正文 |
| `get_draft_count` | 读 | 获取草稿总数 |
| `delete_draft` | 写/确认 | 不可逆删除草稿 |
| `publish_draft` | 写/确认 | 提交异步发布任务 |
| `get_publish_status` | 读 | 查询发布状态和永久链接 |
| `list_published_articles` | 读 | 获取已发布列表，默认不返回正文 |
| `get_published_article` | 读 | 获取已发布文章详情 |
| `delete_published_article` | 写/确认 | 删除单篇或整组已发布文章 |

列表 `count` 范围为 1～20，默认 10；`offset` 默认 0；`no_content` 默认 `true`。

`list_permanent_media` 默认查询图片素材，也可以传入 `media_type` 为 `image`、`voice`、`video` 或 `news`，并使用 `offset`、`count` 分页。例如查询最近 20 张图片：

```json
{
  "media_type": "image",
  "offset": 0,
  "count": 20
}
```

已发布文章删除不会沿用微信 API 中“省略 index 即删除全部”的危险隐式语义：

- `scope=single` 必须提供从 1 开始的 `index`；
- `scope=all` 必须省略 `index`；
- 两种情况都必须 `confirm=true`。

## 常见错误

- `40164 invalid ip`：把 MCP 运行机器的出口 IP 加入公众号后台 IP 白名单。
- `48001 api unauthorized`：确认账号已认证并具备草稿/发布接口权限。
- `40014` / `42001`：服务会清除内存 Token 并自动重试一次；仍失败时检查凭据和公众号后台状态。
- `write tools are disabled`：确认确实需要写操作后设置 `WECHAT_ALLOW_WRITE=true` 并重启 MCP 客户端。
- `uploaded_cover_media_id=...`：封面已经成为永久素材，后续重试应直接复用该 ID。
