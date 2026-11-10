- 新增应用层 SSH/TCP 隧道（`tunnel.open`）：支持端经 tailnet 通道把目标本地
  22 等端口透传到本机，直接 `ssh/scp` 使用，不依赖目标公网端口。
- 新增内置交互式终端（`shell.open`）：macOS/Linux 用 pty.spawn 提供完整终端
  语义，Windows 用交互 cmd；不依赖目标机 sshd，覆盖客户机场景。
- 支持端 CLI 新增 `ssh` / `shell` 子命令；诊断通道端口统一为 17443。
- netd 直连 headscale 超时（目标机 clash fake-ip 未放行 headscale 域名被劫持）
  自动回退走 HTTP 代理重试，提升跨机器注册稳定性。
