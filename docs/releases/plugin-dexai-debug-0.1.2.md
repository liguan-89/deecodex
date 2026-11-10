- 修复支持端连接失败（连接被关闭）：支持端不再直连 tailnet IP（本机通常
  没有 tailnet 路由，流量会被代理劫持），改为经本机 dexai-debug netd 的
  出站代理（127.0.0.1:17445）CONNECT 到目标节点，由 tsnet 内部转发。
- netd 出站代理端口白名单可配置，允许 17443（诊断通道）与 7443（兼容）。
- 支持端连接前自动确保本机 netd 已启动（仅出站代理，不开启被诊断服务）。
- CLI（dexai-diagnose）支持端同样走出站代理转发 tailnet 目标。
- 新增 Windows（windows-amd64）平台二进制，与 macOS Apple Silicon 一并打包。
