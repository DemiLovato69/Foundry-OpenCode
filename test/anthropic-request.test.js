import assert from "node:assert/strict";
import { test } from "node:test";
import { normalizeLmsRequestBody } from "../plugins/lms-responses-compat/request.js";

test("Anthropic cache markers are removed without changing tool data", () => {
  const marker = { type: "ephemeral" };
  const body = {
    cache_control: marker,
    system: [{ type: "text", text: "instructions", cache_control: marker }],
    tools: [{ name: "search", cache_control: marker, input_schema: { type: "object", properties: { cache_control: { type: "string" } } } }],
    messages: [
      { role: "user", content: [{ type: "text", text: "hello", cache_control: marker }] },
      { role: "assistant", content: [{ type: "tool_use", id: "call-1", name: "search", input: { cache_control: "user data" }, cache_control: marker }] },
      { role: "user", content: [{ type: "tool_result", tool_use_id: "call-1", cache_control: marker, content: [{ type: "text", text: "result", cache_control: marker }] }] },
    ],
    thinking: { type: "enabled", budget_tokens: 16000 },
  };

  const result = normalizeLmsRequestBody(body, "anthropic");
  assert.equal(result.cache_control, undefined);
  assert.equal(result.system[0].cache_control, undefined);
  assert.equal(result.tools[0].cache_control, undefined);
  assert.equal(result.messages[0].content[0].cache_control, undefined);
  assert.equal(result.messages[1].content[0].cache_control, undefined);
  assert.equal(result.messages[2].content[0].cache_control, undefined);
  assert.equal(result.messages[2].content[0].content[0].cache_control, undefined);
  assert.deepEqual(result.tools[0].input_schema.properties.cache_control, { type: "string" });
  assert.equal(result.messages[1].content[0].input.cache_control, "user data");
  assert.equal(result.messages[1].content[0].id, "call-1");
  assert.deepEqual(result.thinking, { type: "enabled", budget_tokens: 16000 });
  const once = structuredClone(result);
  normalizeLmsRequestBody(result, "anthropic");
  assert.deepEqual(result, once);
});

test("OpenAI requests retain their own cache_control fields", () => {
  const body = { cache_control: { type: "ephemeral" } };
  assert.deepEqual(normalizeLmsRequestBody(body, "openai"), body);
  assert.equal(normalizeLmsRequestBody(null, "anthropic"), null);
});
