function stripAnthropicFields(value) {
  if (!value || typeof value !== "object") return;
  if (Array.isArray(value)) {
    for (const item of value) stripAnthropicFields(item);
    return;
  }
  delete value.display;
  delete value.eager_input_streaming;
  for (const item of Object.values(value)) stripAnthropicFields(item);
}

function stripCacheBlocks(blocks) {
  if (!Array.isArray(blocks)) return;
  for (const block of blocks) {
    if (!block || typeof block !== "object") continue;
    delete block.cache_control;
    if (block.type === "tool_result") stripCacheBlocks(block.content);
  }
}

function containsInputFile(value) {
  if (!value || typeof value !== "object") return false;
  if (Array.isArray(value)) return value.some(containsInputFile);
  return value.type === "input_file" || Object.values(value).some(containsInputFile);
}

export function normalizeLmsRequestBody(body, provider) {
  if (provider === "anthropic") {
    stripAnthropicFields(body);
    if (!body || typeof body !== "object") return body;
    delete body.cache_control;
    stripCacheBlocks(body.system);
    if (Array.isArray(body.tools)) {
      for (const tool of body.tools) {
        if (tool && typeof tool === "object") delete tool.cache_control;
      }
    }
    if (Array.isArray(body.messages)) {
      for (const message of body.messages) {
        if (!message || typeof message !== "object") continue;
        delete message.cache_control;
        stripCacheBlocks(message.content);
      }
    }
    return body;
  }

  if (containsInputFile(body.input)) {
    throw new Error("Palantir proxy does not support OpenAI input_file attachments");
  }
  for (const tool of body.tools ?? []) {
    if (tool.type !== "function") continue;
    tool.strict ??= false;
    tool.parameters ??= { type: "object", properties: {} };
  }
  delete body.reasoningSummary;
  delete body.textVerbosity;
  if (body.reasoning) delete body.reasoning.summary;
  if (body.text) delete body.text.verbosity;
  return body;
}
