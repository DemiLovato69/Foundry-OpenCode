import { readFileSync, writeFileSync } from "node:fs";

const root = new URL("./node_modules/opencode-rag-plugin/dist/", import.meta.url);
function replace(file, oldText, newText) {
  const url = new URL(file, root);
  const text = readFileSync(url, "utf8");
  if (text.includes(newText)) return;
  if (!text.includes(oldText)) throw new Error(`RAG ${file} changed; refusing an unverified patch`);
  writeFileSync(url, text.replace(oldText, newText));
}
const provider = new URL("embedder/openai.js", root);
const source = readFileSync(provider, "utf8");
if (source.includes("body.input_type")) {
  const patched = source.replace(/\s*if \(purpose\) \{\s*body\.input_type = this\.toInputType\(purpose\);\s*\}/, "");
  if (patched.includes("body.input_type")) throw new Error("Unable to patch Foundry embeddings input_type");
  writeFileSync(provider, patched);
}
replace("vectorstore/lancedb.js", "if (!options?.skipIndex) {", 'if (!options?.skipIndex && process.env.OPENCODE_RAG_DISABLE_ANN !== "1") {');
replace("vectorstore/lancedb.js", "await this.withWriteLock(() => this.ensureCosineIndex());", 'if (process.env.OPENCODE_RAG_DISABLE_ANN !== "1") { await this.withWriteLock(() => this.ensureCosineIndex()); }');
replace("vectorstore/lancedb.js", "const oldPath = `${realPath}_old`;", "const oldPath = `${realPath}_old-${process.pid}-${Date.now()}`;");
const pipeline = readFileSync(new URL("indexer/pipeline.js", root), "utf8");
if (!pipeline.includes("if (existingCount > 0 && Object.keys(manifest.files).length === 0)")) {
  replace("indexer/pipeline.js", "const existingCount = await options.store.count();", 'const existingCount = await options.store.count();\n    if (existingCount > 0 && Object.keys(manifest.files).length === 0) { manifestStatus = "missing"; }');
}
if (!pipeline.includes("!aborted() && workspaceFiles.length > 0 && stats.totalChunks === 0")) {
  replace("indexer/pipeline.js", "if (tempStorePath) {\n        if (!aborted()) {", 'if (tempStorePath) {\n        if (!aborted() && workspaceFiles.length > 0 && stats.totalChunks === 0) { throw new Error("RAG rebuild produced no embeddings; keeping the existing index"); }\n        if (!aborted()) {');
}
