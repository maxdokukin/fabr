# Bright Data

Bright Data is an optional public-web tool. Fabr plans, approves, and records the task the same way it does for a local file. Visa is not part of this integration.

## Credential

Put the token in `~/.fabric-space/secrets.toml` and keep that file mode `600`. Do not commit it.

```toml
brightdata_api_token = "YOUR_API_TOKEN"
```

The same value may instead be provided as `BRIGHTDATA_API_TOKEN` or `API_TOKEN` in the process environment. The desktop app reads it in the Rust backend. It is not bundled into the frontend.

The official remote MCP server is `https://mcp.brightdata.com/mcp`. Fabr sends the token as the `token` query parameter on that request and does not write the token into the task record, the research note, or logs.

`FABR_BRIGHTDATA_MCP_URL` overrides the server address for tests. Leave it unset for the real server.

## Tools

When the token is present and the task grant includes `external_action`, the planner may request:

- `bright_data_search` calls the MCP tool `search_engine` for a public query.
- `bright_data_extract` calls `scrape_as_markdown` for one public `http` or `https` URL.

Both append a section to `research.md` in the task's folder. Each section has `source: bright_data`, `retrieved_at`, source URLs, snippets, and `error` when the call is empty, rate limited, timed out, paywalled, or unavailable. An empty search result is recorded as `error: empty` and does not invent hits. An extract records the requested public URL plus links found on that page. GitHub marketing links are not kept as hits. The page text is redacted before it is written.

Login, account, checkout, and paywall URLs are refused before the request. Queries that contain an email address or a social-security shape are refused. Private hosts are refused.

Without a token, those tools are not offered and a local `write_file` task still runs.

The lifecycle test uses a stand-in MCP server. `live_public_docs_and_issues` is `#[ignore]` and passed on this v0.7.0 port: the task waited for approval, then wrote `research.md`. `docs/brightdata-live.md` keeps the older page capture beneath a note for that run.
