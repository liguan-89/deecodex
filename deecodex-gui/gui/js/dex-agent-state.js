// DEX Agent 状态机小工具：保持为纯函数，便于 Node 级回归测试。
(function(root) {
  function shouldStopAfterToolResult(toolResult) {
    return !!(toolResult && toolResult.cancelled);
  }

  function removePendingAssistantToolCall(messages) {
    if (!Array.isArray(messages) || messages.length === 0) return messages;
    var last = messages[messages.length - 1];
    if (last && last.role === 'assistant' && Array.isArray(last.tool_calls) && last.tool_calls.length) {
      messages.pop();
    }
    return messages;
  }

  var api = {
    shouldStopAfterToolResult: shouldStopAfterToolResult,
    removePendingAssistantToolCall: removePendingAssistantToolCall
  };

  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  root.DexAgentState = api;
})(typeof window !== 'undefined' ? window : globalThis);
