import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

test("project RAG uses the assigned private token rather than an inherited stack token", () => {
  const home = mkdtempSync(join(tmpdir(), "foundry-rag-token-"));
  try {
    const project = join(home, "repo");
    const cli = join(project, ".opencode/node_modules/opencode-rag-plugin/dist/cli/index.js");
    const token = join(home, ".config/foundry-opencode/tokens/default");
    mkdirSync(dirname(cli), { recursive: true });
    mkdirSync(dirname(token), { recursive: true });
    mkdirSync(join(home, ".opencode"), { recursive: true });
    writeFileSync(join(home, ".opencode/token"), "wrong-parent-token");
    writeFileSync(token, "correct-project-token", { mode: 0o600 });
    writeFileSync(cli, 'if (process.env.OPENAI_API_KEY !== "correct-project-token") process.exit(1); console.log("token-ok")');
    copyFileSync(fileURLToPath(new URL("../foundry-opencode/src/rag-cli.mjs", import.meta.url)), join(project, ".opencode/rag-cli.mjs"));
    writeFileSync(join(project, ".opencode/opencode.json"), JSON.stringify({ providers: { "palantir-openai": { settings: { apiKey: `{file:${token}}` } } } }));
    const { FOUNDRY_TOKEN, OPENAI_API_KEY, OPENCODE_API_KEY, ...env } = process.env;
    const result = spawnSync(process.execPath, [".opencode/rag-cli.mjs", "index"], { cwd: project, env, encoding: "utf8" });
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /token-ok/);
    assert.ok(!readFileSync(join(project, ".opencode/opencode.json"), "utf8").includes("correct-project-token"));
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
});
