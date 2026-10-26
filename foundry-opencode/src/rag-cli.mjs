import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

process.env.LANCEDB_LOG ??= "warn,lance::index::vector::builder=error";
process.env.OPENCODE_RAG_DISABLE_ANN ??= "1";
const token = process.env.FOUNDRY_TOKEN || process.env.OPENAI_API_KEY || process.env.OPENCODE_API_KEY;
if (!token) throw new Error("RAG needs OPENCODE_API_KEY from the project/parent direnv environment");
process.env.OPENAI_API_KEY = token;
const cli = new URL("./node_modules/opencode-rag-plugin/dist/cli/index.js", import.meta.url);
const result = spawnSync(process.execPath, [fileURLToPath(cli), ...process.argv.slice(2)], { cwd: process.cwd(), env: process.env, stdio: "inherit" });
if (result.error) throw result.error;
process.exit(result.status ?? 1);
