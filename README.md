# OpenCode v2 Config

Personal OpenCode configuration for OpenAI and Anthropic models through the Palantir proxy. Requires OpenCode v2; this migration is tested with 2.0.32. The configuration uses the native v2 format and must not be loaded by v1.

## Install

Remove a package-managed v1 installation before installing v2. Follow the [official v2 installation instructions](https://opencode.ai/v2/docs/cli/), then confirm `opencode --version` reports v2.

Clone this repository into `~/.config/opencode`, backing up any existing directory first:

```sh
mkdir -p ~/.config
git clone <repo-url> ~/.config/opencode
cd ~/.config/opencode
npm install
```

When copying files into an existing installation, copy `opencode.jsonc`, `package.json`, `agents/`, `commands/`, and `plugins/`. Remove the previous active v1 plugin files; v2 discovers local plugins automatically. `plugins-old/` holds the original v1 plugins and the incomplete copied Ponytail plugin for reference; OpenCode does not load that directory.

## Credentials

Set these variables in the environment that starts the OpenCode server, for example through `~/.zshrc`:

```sh
export OPENCODE_BASE_URL="https://your-foundry-stack.palantir.com"
export OPENCODE_API_KEY="your-foundry-token"
```

Keep the token out of this repository. Check that variables are present without printing their values:

```sh
test -n "$OPENCODE_BASE_URL" && echo "OPENCODE_BASE_URL is set"
test -n "$OPENCODE_API_KEY" && echo "OPENCODE_API_KEY is set"
```

The current OpenAI endpoint is `${OPENCODE_BASE_URL}/language-model-service/api/proxy/openai/v1`. Anthropic keeps the v1 setup's `${OPENCODE_BASE_URL}/api/v2/llm/proxy/anthropic/v1`. Adjust `providers.<id>.settings.baseURL` if your Foundry enrollment uses different routes.

## Models and plugins

Start `opencode` in a project. The default model is `palantir-openai/gpt-6-sol`; the built-in title agent uses `palantir-openai/gpt-6-luna`. Provider policies allow only `palantir-openai` and `palantir-anthropic`. These dedicated IDs inherit the vendor catalogs through `canonical` and avoid binding proxy requests to saved OpenAI or Anthropic OAuth accounts. Existing sessions using `openai/...` or `anthropic/...` must select the corresponding `palantir-*` model through `/models`. Automatic updates and sharing are disabled.

OpenAI uses the native Responses runtime, and Anthropic uses the native Messages runtime. `modelID` values in `opencode.jsonc` contain the upstream Palantir model identifiers. The local `lms-responses-compat` plugin uses v2 `http.request` hooks to remove unsupported fields and normalize bearer authentication for primary, compaction, title, and transient requests. It rejects OpenAI `input_file` attachments, including PDFs; use text or image inputs there. Anthropic retains PDF support.

The compatibility plugin also preserves the old model allowlists through a v2 model transform. When adding a model, update both `providers.<id>.models` in `opencode.jsonc` and `plugins/lms-responses-compat/models.json`.

[Ponytail](https://www.npmjs.com/package/@dietrichgebert/ponytail) is pinned to 5.3.2, whose published package includes its v2 entrypoint, shared hooks, commands, and skills. Its default mode is full. Use `/ponytail off`, `/ponytail lite`, `/ponytail`, or `/ponytail ultra` to switch modes, and `/ponytail-help` for help. Active mode persists in `~/.config/opencode/.ponytail-active`. To disable loading Ponytail entirely, replace its plugin entry with `-ponytail` (the plugin ID).

Local command definitions live in `commands/`; custom primary agents live in `agents/`. The old GitKraken hook remains archived and inactive.

V2 accepts the retained `lsp` settings but does not run language servers, expose LSP tools, or provide LSP diagnostics. Use each project's lint, typecheck, and compiler commands instead. Shell and edit permissions remain allowed; access outside the project asks for permission, preserving v1's default.

OpenCode v2 watches server configuration automatically. Use `opencode reload` after changing plugin code; restart the server after changing its environment. Terminal settings belong to global `cli.json`, which v2 migrates automatically and this repo does not track.

Clients that call the v1 server API, including older OpenChamber versions, must migrate to the v2 API and `@opencode/client`; updating this config does not migrate those clients.

A global `integration.use` policy blocks `plugin:context-mode`, whose current package uses the incompatible v1 plugin API. This also blocks the inherited entry in `~/.opencode/opencode.json` without modifying that file. Remove the policy after installing a v2-compatible release.

## Palantir MCP

The Palantir MCP server is configured under `mcp.servers.palantir` and starts disabled. Change `disabled` to `false` to enable it. It requires Node.js 18 or newer and access to your Foundry enrollment.

The command maps `OPENCODE_API_KEY` to `FOUNDRY_TOKEN` for the MCP process:

```jsonc
{
  "mcp": {
    "servers": {
      "palantir": {
        "type": "local",
        "command": [
          "sh",
          "-c",
          "FOUNDRY_TOKEN=\"$OPENCODE_API_KEY\" exec npx -y palantir-mcp --foundry-api-url \"$OPENCODE_BASE_URL\""
        ],
        "disabled": false
      }
    }
  }
}
```

## Verify

```sh
npm test
opencode debug agents
opencode plugin list
opencode models
```

In a project, verify that both custom agents and the Ponytail commands appear, select a model, and send a short prompt. Confirm real proxy access separately for OpenAI and Anthropic, and enable and exercise MCP if needed. Local tests use synthetic requests and do not spend model tokens.

Validation included nine local tests, the official v2 JSON schema, the 36-model allowlist, Ponytail commands and mode persistence, and complete OpenAI and Anthropic prompts through OpenCode 2.0.32 against a local mock proxy using the two custom agents. A real `palantir-openai/gpt-6-sol` prompt through Foundry returned `OK`. Real Anthropic and MCP connectivity remain untested.

Migration references: [configuration](https://opencode.ai/v2/docs/migrate-v1/) and [plugins](https://opencode.ai/v2/docs/build/plugins/migrate-v1/).
