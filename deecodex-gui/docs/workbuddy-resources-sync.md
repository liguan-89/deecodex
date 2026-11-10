# WorkBuddy 资源提取与更新手册

> 用途：从本机 WorkBuddy（腾讯 AI 桌面工作台）提取 **Skill / MCP 连接器 / 专家（Agent）** 资源，
> 供 DEX AI 插件市场的 Skill 专区 / MCP 专区 / 深度研究专区使用，并同步到 CDN 服务器。
> 维护者每次「更新内容」时按本手册执行。

## 0. 资源总览与存放位置

本地提取目标：`~/.deecodex/wb-resources/`

```
wb-resources/
├── skills/                # 594 个本地技能包（SKILL.md + scripts/references/assets）
├── skills-icons/          # 技能图标（官方 385 + 生成 letter 占位 817 = 1202）
├── skills-cache.json      # WorkBuddy 技能中心清单（657 个，含中文名/介绍/示例/图标/排序）
├── skills-icons.json      # 技能名 -> 图标文件名 映射
├── connectors/            # 103 个 MCP/CLI 连接器（mcp.json / cli.json / skills/）
├── connectors-icons/      # 114 个连接器图标
├── connectors-index.json  # 连接器中文索引（name/description_zh/examples_zh）
├── agents/                # 410 个专家（agents/*.md prompt + avatars/ 头像）
└── agents-index.json      # 专家清单（expert_center.json，中文元数据）
```

CDN 线上位置：`https://api.liguan.me/releases/dex-ai/wb-resources/`
服务器：`<DEPLOY_HOST>`（ssh key `<SSH_KEY_PATH>`），静态根 `/var/www/dex-ai/releases/dex-ai`

## 1. 各资源的数据源

| 资源 | 数据源 | 类型 |
|---|---|---|
| **专家清单** | 腾讯云 COS `https://acc-1258344699.cos.accelerate.myqcloud.com/workbuddy/expert-marketplace/expert_center.json` | 公开静态 JSON（410 专家，含 displayName/profession/description/tags/quickPrompts） |
| **专家 prompt** | 同 COS `.../expert-marketplace/plugins/<plugin>/agents/*.md` | 公开静态 Markdown（410 个） |
| **专家头像** | 同 COS `.../expert-marketplace/avatars/*.png` | 公开静态 PNG（408 个，512×512） |
| **技能清单** | **WorkBuddy 应用 Local Storage**（leveldb）的 `skills-cache-builtin-market` | 本地缓存 JSON（UTF-16LE + snappy 压缩）——需先在 WorkBuddy 界面浏览一遍技能触发缓存 |
| **技能图标** | `https://openplatform-cdn.codebuddy.cn/public/skills/icons/<name>.<ext>` | 公开 CDN（657 个中 385 个有真实图标；其余 272 个为 `letter:auto` 占位，用生成 SVG） |
| **连接器** | `~/.workbuddy/connectors-marketplace/connectors/<id>/` | 本地目录（mcp.json / cli.json / skills/） |
| **连接器中文索引** | `~/.workbuddy/connectors-marketplace/.codebuddy-connector/connectors.json` | 本地 JSON（name/description_zh/examples_zh） |
| **连接器图标** | `~/.workbuddy/connectors-marketplace/icons/` | 本地 SVG/PNG（114 个） |

## 2. 一键提取脚本

`scripts/import-workbuddy-resources.sh` 已覆盖连接器 / 专家 / 技能 / 图标：

```bash
bash scripts/import-workbuddy-resources.sh
```

脚本行为：
1. **连接器**：复制 `connectors-marketplace/connectors/*` → `wb-resources/connectors/`；复制 `connectors.json` → `connectors-index.json`；复制 `icons/` → `connectors-icons/`
2. **专家**：下载 COS `expert_center.json` → `agents-index.json`；逐专家下载 prompt 到 `agents/<plugin>/agents/*.md`、头像到 `agents/<plugin>/avatars/*.png`（幂等，已存在跳过）
3. **技能**：复制内置技能缓存 → `skills/builtin-*`；从官方插件市场 `plugins/`、`external_plugins/`、`cb_teams_marketplace/` 复制技能包（带 `source-plugin.json` 中文描述）

## 3. 技能清单（skills-cache.json）提取——特殊步骤

技能中心的**全量清单不在公开接口**，缓存在 WorkBuddy 应用里，必须：

### 3.1 触发缓存
打开本机 WorkBuddy → 技能中心，**把技能列表完整浏览一遍**（滚动加载），Local Storage 会写入
`skills-cache-builtin-market`（键含 UTF-16LE 值）。

### 3.2 从 leveldb 解析
缓存位于 `~/.workbuddy/app/session/Local Storage/leveldb/*.ldb`，数据经 **snappy 压缩 + UTF-16BE/LE 编码**。
关键提取逻辑（Python，需 `pip install python-snappy`）：

```python
import os, json, snappy

def read_varint(data, pos):
    shift = 0; result = 0
    while True:
        b = data[pos]; pos += 1
        result |= (b & 0x7f) << shift
        if not (b & 0x80): break
        shift += 7
    return result, pos

def extract_skill_cache(ldb_path):
    data = open(ldb_path, 'rb').read()
    footer = data[-48:]
    # footer: metaindex handle + index handle（BlockHandle = varint offset + varint size）+ padding + magic
    o, p = read_varint(footer, 0); s, p = read_varint(footer, p)  # metaindex
    i_off, p = read_varint(footer, p); i_size, _ = read_varint(footer, p)  # index
    iraw = data[i_off:i_off+i_size]
    if data[i_off+i_size] == 1:  # snappy
        iraw = snappy.decompress(iraw)
    # 遍历 index entries，拿 data block handle
    # 对每个 data block：解压（trailer type 在 block 之后），遍历 entries
    # key 命中 "skills-cache-builtin-market" 的 value 即技能清单
    # value 解码：先去掉开头 0..4 个多余字节，UTF-16LE decode，再 json.loads
    ...
```

完整解析器以一次成功提取为准（snappy 块 + UTF-16LE + 去掉头部多余字节）。提取结果写入
`wb-resources/skills-cache.json`，结构：

```json
{ "skills-cache-builtin-market": [ { "skill_id": "...", "icon": "...", "name": "...",
    "display_name_zh": "中文名", "description_zh": "中文介绍", "examples_zh": [...],
    "categories": [...], "featured": 0|1, "sort_order": n, "featured_order": n, ... } ] }
```

### 3.3 技能图标
- 真实图标：按 `name` 从 `openplatform-cdn.codebuddy.cn/public/skills/icons/<name>.<ext>` 下载到 `skills-icons/`
- `letter:auto` 占位技能：用脚本生成字母 SVG（圆形 + 首字 + 分类配色）到 `skills-icons/letter-<name>.svg`
- 生成 `skills-icons.json`（技能名 → 图标文件名）

## 4. 更新流程（下次要更新内容时）

```bash
# 1. 在 WorkBuddy 界面浏览技能中心（若技能清单要更新）

# 2. 重新提取
bash scripts/import-workbuddy-resources.sh

# 3. 若技能清单有变，重新解析 skills-cache.json（见第 3 节）

# 4. 生成本地数据（验证技能总数/图标/中文名）
#    cargo build 后由 list_wb_resources 读取验证

# 5. 同步到 CDN 服务器（api.liguan.me / Mobile Codex 服务器）
rsync -az -e "ssh -i <SSH_KEY_PATH>" \
  ~/.deecodex/wb-resources/ \
  <DEPLOY_HOST>:/var/www/dex-ai/releases/dex-ai/wb-resources/

# 6. 客户端自动生效：DEX AI 进入插件市场时 sync_wb_resources 拉清单，
#    图标/头像按需 fetch_wb_asset 下载；本地缓存按 mtime 自动失效
```

## 5. 客户端读取与缓存机制

- `list_wb_resources`（Tauri 命令）：扫描 `wb-resources` 返回连接器/技能/专家；**结果缓存**按关键文件
  mtime 失效（首次 ~440ms，命中 ~3ms）
- `sync_wb_resources`：从 CDN 下载 4 个清单（skills-cache / skills-icons / agents-index / connectors-index）
- `fetch_wb_asset`：按需下载单个资源（图标/头像/mcp.json）到本地，幂等
- `install_wb_connectors / install_wb_skills / install_wb_agents`：一键安装到 Codex / Claude Code / opencode

## 6. 已知边界

- **技能中心清单**依赖本机 WorkBuddy 浏览缓存，不是公开接口；更新前必须先在 WorkBuddy 里浏览一遍技能
- COS 桶不允许匿名列目录，只能按已知文件名下载；`expert_center.json` 是唯一公开索引
- 技能图标有 272 个为 `letter:auto`（官方无独立图标文件），用生成 SVG 占位
- CLI 型连接器（飞书 lark-cli 等）需手动安装命令行工具；OAuth 凭据在 WorkBuddy 侧加密，不迁移
