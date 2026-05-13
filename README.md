# 编译-mac

## 职责

macOS 平台二进制编译工作区。

## 编译

```bash
cargo build --release          # 产物在 target-mac/release/
```

## 目标平台

- macOS ARM64 (Apple Silicon)
- 产物格式：`.dmg`

## 发布

```bash
# 打发布 tag（英文）
git tag release/v<版本号>

# 推送到公开仓库
git push origin release/v<版本号>

# 上传产物到 Releases
# https://github.com/liguan-89/deecodex/releases
```

## 注意

- 此工作区对应公开仓库 `origin`（`liguan-89/deecodex`）
- 源码分支为 `deecodex-gui`，编译时有平台差异需在此分支上调整
- `target/` 已隔离为 `target-mac/`，与其他编译工作区互不干扰
