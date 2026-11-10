- 修复 tailnet 节点间数据路径不通：headscale 控制面与 DERP 加入 NO_PROXY
  直连，避免走系统代理（clash）时 DERP WebSocket 被破坏（表现为节点在线
  但互连超时/连接失败）。
