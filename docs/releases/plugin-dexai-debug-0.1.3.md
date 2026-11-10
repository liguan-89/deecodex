- 修复 netd 启动失败（authkey expired）：已注册节点直接用持久化身份恢复，
  不再复用已消费/过期的一次性 auth key；注册成功自动清理 auth key 文件。
- 支持端连接（ensure_netd）等待 netd 注册完成（最多 15 秒），并支持
  force 参数在身份失效时强制重新注册。
