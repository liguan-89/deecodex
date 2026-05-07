# rollout 019dfe49 integration test plan

本文件记录 rollout `019dfe49` 分析得出的 P0/P1 集成测试草案。当前只作为测试辅助文档落在 `tests/` 下，避免和正在改 `src/stream.rs` / `src/translate.rs` 的 worker 冲突。

## P0: sequence_number

- Start the router with a deterministic mock upstream SSE response.
- POST `/v1/responses` with `stream: true`.
- Parse every SSE event that contains `sequence_number`.
- Assert the numbers are strictly increasing and unique for the full stream.
- Assert terminal events such as `response.completed` or failure events also carry the expected sequence position.
- Repeat the same request to hit cache replay and assert replayed events keep a valid sequence.

Status: partially implemented. Live stream, cached stream, and retrieve stream replay now assert monotonic `sequence_number`; retrieve replay also validates `starting_after` cursor behavior.

## P0: response echo

- Create a non-streaming response with `store: true`, `metadata`, and a stable model name.
- Retrieve it with `GET /v1/responses/{id}`.
- Compare `id`, `model`, `status`, `output`, `usage`, `metadata`, and `truncation`.
- For streaming, compare the final `response.completed.response` payload with the stored retrieve result.
- For interrupted upstream streams, assert the response is not stored as `completed`.

Status: partially implemented. Retrieve stream replay now checks stored response echo for `id`, `metadata`, `usage`, and output item identity. Non-streaming create/retrieve deep equality is still a next-step test.

## P0: output item id

- Use a mock upstream message with normal text output and a function call output.
- Assert `response.output_item.added.item.id`, related delta/done events, final response `output[].id`, and retrieve `output[].id` match.
- Repeat through cache replay and assert IDs are replayed from stored output, not regenerated.
- Extend the same shape to `computer_call`, `local_mcp_call`, and future `file_search_call` items when those paths are executable.

Status: partially implemented. Retrieve stream replay validates message item IDs across added/delta/done/completed. Stream tool-call IDs and cached reasoning shape are covered by integration/unit tests.

## P1: include

- Send supported include values that can be generated locally, such as `output_text`, `usage`, and input item listing behavior.
- Send unsupported hosted-only include values and assert the behavior is explicit: clear unsupported error or documented predictable ignore.
- Assert include handling does not change stored response identity or output item IDs.

Status: partially implemented. Create-time unsupported include and local `file_search_call.results` are covered. Retrieve-time include behavior still needs explicit tests.

## P1: file_search_call

- Upload two small text files through `/v1/files`.
- Create a vector store and attach the files.
- Send a Responses request with `file_search` constrained to that vector store.
- Assert retrieved metadata contains the query, vector store IDs, file IDs, and matched snippets.
- Once implemented, assert output contains a `file_search_call` item and that retrieve/input_items expose the same search evidence.

Status: partially implemented. Local `file_search_call` output and metadata are covered for create. Retrieve/input_items evidence-chain tests remain next.
