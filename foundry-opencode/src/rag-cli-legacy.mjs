import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";

process.env.LANCEDB_LOG ??= "warn,lance::index::vector::builder=error";
process.env.OPENCODE_RAG_DISABLE_ANN ??= "1";
let token;
for (let dir = process.cwd(); ; dir = dirname(dir)) {
  const file = join(dir, ".opencode/token");
  if (existsSync(file)) { token = readFileSync(file, "utf8"); break; }
  if (dirname(dir) === dir) break;
}
token ||= process.env.FOUNDRY_TOKEN || process.env.OPENAI_API_KEY || process.env.OPENCODE_API_KEY;
if (!token) throw new Error("RAG needs a stack .opencode/token; run foundry-opencode init from the stack directory");
process.env.OPENAI_API_KEY = token;
const cli = new URL("./node_modules/opencode-rag-plugin/dist/cli/index.js", import.meta.url);
const result = spawnSync(process.execPath, [fileURLToPath(cli), ...process.argv.slice(2)], { cwd: process.cwd(), env: process.env, stdio: "inherit" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
