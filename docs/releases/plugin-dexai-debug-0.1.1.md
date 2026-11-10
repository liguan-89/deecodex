- 远程通道改用 netd grpc-target 端口（17443）：tailnet 对外端口 = netd
  --grpc-target 端口（入站转发到 localhost 同端口）。之前用 7779 时
  tailnet 入站无监听，支持端连接卡在 SYN_SENT / 一直"正在连接"。
- 支持端连接增加 10 秒超时，避免连接长时间无响应。
