#!/usr/bin/env bash
# 提取 WorkBuddy 的专家/技能/连接器资源到 DEX AI 可控目录（~/.deecodex/wb-resources）
# 覆盖：
#   - 连接器：connectors-marketplace（103 个目录 + connectors.json 中文索引）
#   - 专家：从 WorkBuddy 专家市场（腾讯云 COS）下载 429 个专家（元数据+prompt+头像）
#   - 技能：WorkBuddy 内置 + CodeBuddy 官方插件市场（plugins/ + external_plugins/）+ Teams 市场
# 用法: bash scripts/import-workbuddy-resources.sh
set -euo pipefail

WB_HOME="${HOME}/.workbuddy"
DEST="${HOME}/.deecodex/wb-resources"
EXP_BASE="https://acc-1258344699.cos.accelerate.myqcloud.com/workbuddy/expert-marketplace"

mkdir -p "${DEST}/connectors" "${DEST}/skills" "${DEST}/agents"

echo "== 1/4 连接器 connectors =="
SRC_CONN="${WB_HOME}/connectors-marketplace/connectors"
if [ -d "${SRC_CONN}" ]; then
  count=0
  for dir in "${SRC_CONN}"/*/; do
    [ -d "$dir" ] || continue
    name=$(basename "$dir")
    cp -R "$dir" "${DEST}/connectors/${name}"
    count=$((count + 1))
  done
  echo "已复制 ${count} 个连接器"
else
  echo "未找到连接器市场目录: ${SRC_CONN}"
fi

# 连接器中文索引（name/description_zh/examples_zh）
if [ -f "${WB_HOME}/connectors-marketplace/.codebuddy-connector/connectors.json" ]; then
  cp "${WB_HOME}/connectors-marketplace/.codebuddy-connector/connectors.json" "${DEST}/connectors-index.json"
  echo "已复制连接器中文索引 connectors-index.json"
else
  echo "未找到连接器中文索引"
fi

# 连接器图标（SVG/PNG）
if [ -d "${WB_HOME}/connectors-marketplace/icons" ]; then
  mkdir -p "${DEST}/connectors-icons"
  cp -R "${WB_HOME}/connectors-marketplace/icons/." "${DEST}/connectors-icons/"
  echo "已复制连接器图标 $(ls "${DEST}/connectors-icons" | wc -l | tr -d ' ') 个"
else
  echo "未找到连接器图标目录"
fi

echo "== 2/4 专家 agents（从 WorkBuddy 专家市场下载 429 个）=="
EXP_MANIFEST="${DEST}/agents-index.json"
if [ ! -f "${EXP_MANIFEST}" ] || [ ! -s "${EXP_MANIFEST}" ]; then
  curl -s -m 30 -o "${EXP_MANIFEST}" "${EXP_BASE}/expert_center.json" || echo "manifest 下载失败"
fi
if [ -s "${EXP_MANIFEST}" ]; then
  # 保留本地已下载的专家包（如 we-chat-mini-program-developer）
  cp -R "${WB_HOME}/plugins/marketplaces/experts/plugins/"* "${DEST}/agents/" 2>/dev/null || true
  TOTAL=$(python3 -c "import json;print(len(json.load(open('${EXP_MANIFEST}'))['experts']))" 2>/dev/null || echo 0)
  echo "manifest 专家总数: ${TOTAL}"
  OK=0; FAIL=0
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    plugin=$(echo "$line" | cut -d'|' -f1)
    prompt=$(echo "$line" | cut -d'|' -f2)
    avatar=$(echo "$line" | cut -d'|' -f3)
    [ -z "$plugin" ] && continue
    dir="${DEST}/agents/${plugin}"
    mkdir -p "${dir}/agents" "${dir}/avatars"
    # prompt 文件
    pfile="${dir}/agents/$(basename "$prompt")"
    if [ -n "$prompt" ]; then
      if [ -s "$pfile" ]; then
        OK=$((OK + 1))
      else
        if curl -s -m 20 -o "$pfile" "${EXP_BASE}${prompt}" && [ -s "$pfile" ]; then
          OK=$((OK + 1))
        else
          rm -f "$pfile" 2>/dev/null || true
          FAIL=$((FAIL + 1))
        fi
      fi
    fi
    # 头像
    if [ -n "$avatar" ]; then
      afile="${dir}/avatars/$(basename "$avatar")"
      if [ ! -s "$afile" ]; then
        curl -s -m 20 -o "$afile" "${EXP_BASE}${avatar}" || rm -f "$afile" 2>/dev/null || true
      fi
    fi
  done < <(python3 - "$EXP_MANIFEST" <<'PYEOF'
import json, sys
d = json.load(open(sys.argv[1]))
for e in d["experts"]:
    prompt = e.get("promptFile", "")
    if not prompt.startswith("/"):
        prompt = "/" + prompt
    parts = prompt.split("/")
    plugin = e.get("plugin") or (parts[2] if len(parts) > 2 else e["id"])
    avatar = e.get("avatar", "") or ""
    print(f"{plugin}|{prompt}|{avatar}")
PYEOF
)
  echo "专家 prompt 下载完成: 成功 ${OK}, 失败 ${FAIL}"
else
  echo "专家 manifest 为空，跳过下载"
fi

echo "== 3/4 技能 skills =="
# 3a. WorkBuddy 内置技能（已缓存的解包版本）
SRC_BUILTIN="${WB_HOME}/plugins/cache/workbuddy-builtin"
if [ -d "${SRC_BUILTIN}" ]; then
  count=0
  for dir in "${SRC_BUILTIN}"/skill-*/; do
    [ -d "$dir" ] || continue
    pkg=$(basename "$dir")
    ver_dir=$(ls -d "${dir}"*/ 2>/dev/null | head -1 || true)
    [ -n "$ver_dir" ] && [ -d "$ver_dir" ] || continue
    cp -R "${ver_dir}." "${DEST}/skills/builtin-${pkg}"
    if [ -f "${ver_dir}.codebuddy-plugin/plugin.json" ] && [ ! -f "${DEST}/skills/builtin-${pkg}/source-plugin.json" ]; then
      cp "${ver_dir}.codebuddy-plugin/plugin.json" "${DEST}/skills/builtin-${pkg}/source-plugin.json"
    fi
    count=$((count + 1))
  done
  echo "已复制 ${count} 个内置技能"
fi

# 3b. CodeBuddy 官方插件市场（plugins/ + external_plugins/）+ Teams 市场
copy_skill_root() {
  local root="$1" prefix="$2"
  [ -d "$root" ] || return 0
  local count=0
  while IFS= read -r skill_dir; do
    [ -d "$skill_dir" ] || continue
    local plugin skill target
    plugin=$(basename "$(dirname "$(dirname "$skill_dir")")")
    skill=$(basename "$skill_dir")
    target="${DEST}/skills/${prefix}-${plugin}-${skill}"
    if [ ! -e "$target" ]; then
      cp -R "$skill_dir" "${target}"
    fi
    # 附带来源插件元数据（中文 name/description），供后端展示
    local plugin_json="${root}/${plugin}/.codebuddy-plugin/plugin.json"
    if [ -f "$plugin_json" ] && [ ! -f "${target}/source-plugin.json" ]; then
      cp "$plugin_json" "${target}/source-plugin.json"
    fi
    count=$((count + 1))
  done < <(find "$root" -name SKILL.md -exec dirname {} \; | sort -u)
  echo "  已复制 ${count} 个技能（${prefix}）"
}

echo "  - 官方插件市场 plugins/"
copy_skill_root "${WB_HOME}/plugins/marketplaces/codebuddy-plugins-official/plugins" "market"
echo "  - 官方插件市场 external_plugins/"
copy_skill_root "${WB_HOME}/plugins/marketplaces/codebuddy-plugins-official/external_plugins" "ext"
echo "  - Teams 市场 cb_teams_marketplace/"
copy_skill_root "${WB_HOME}/plugins/marketplaces/cb_teams_marketplace/plugins" "teams"

echo "== 4/4 汇总 =="
du -sh "${DEST}" 2>/dev/null || true
echo "--- 统计 ---"
echo "连接器: $(ls "${DEST}/connectors" 2>/dev/null | wc -l | tr -d ' ')"
echo "技能:   $(ls "${DEST}/skills" 2>/dev/null | wc -l | tr -d ' ')"
echo "专家:   $(ls "${DEST}/agents" 2>/dev/null | wc -l | tr -d ' ')"
if [ -f "${DEST}/agents-index.json" ]; then
  echo "专家 manifest 条目: $(python3 -c "import json;print(len(json.load(open('${DEST}/agents-index.json'))['experts']))" 2>/dev/null || echo '?')"
fi
