import { readFileSync } from "node:fs";
import { normalizeLmsRequestBody } from "./request.js";

const allowedModels = JSON.parse(readFileSync(new URL("./models.json", import.meta.url), "utf8"));

export default {
  id: "lms-responses-compat",

  async setup(ctx) {
    // V2 ignores V1 provider whitelists. Limit the catalog to configured models.
    await ctx.model.transform((editor) => {
      for (const [providerID, models] of Object.entries(allowedModels)) {
        for (const model of editor.list(providerID)) {
          if (!models.includes(model.id)) editor.remove(providerID, model.id);
        }
      }
    });

    await ctx.session.hook("http.request", async (event) => {
      const providerID = event.model.providerID;
      if (!(providerID in allowedModels)) return;
      // Client reads return { location, data }; resolve current settings per request.
      const { data: provider } = await ctx.provider.get({ providerID });
      if (!provider.settings.baseURL) return;
      const base = new URL(provider.settings.baseURL);

      const request = event.request;
      const url = new URL(request.url);
      const endpoint = providerID === "palantir-openai" ? "responses" : "messages";
      if (request.method !== "POST" || url.origin !== base.origin ||
          url.pathname !== `${base.pathname.replace(/\/+$/, "")}/${endpoint}`) return;

      const headers = new Headers(request.headers);

      // Read a clone so malformed JSON can pass through without consuming the body.
      let body;
      try {
        body = await request.clone().json();
      } catch {
        event.request = new Request(request, { headers });
        return;
      }

      body = JSON.stringify(normalizeLmsRequestBody(body, providerID === "palantir-anthropic" ? "anthropic" : "openai"));
      headers.delete("content-length");
      event.request = new Request(request, { headers, body });
    });
  },
};
