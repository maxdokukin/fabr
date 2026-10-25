# Sponsor credentials

Secret values stay in the trusted backend (`<fabric-space>/secrets.toml` or the process environment). The planner receives tool names, input schemas, and enabled or disabled. Key values, tokens, and secret paths are not put in the plan prompt, the approval digest, the task log, or research notes.

Loaders accept these names, plus the aliases already stored in the local file: `moss_project_id` / `MOSS_PROJECT_ID`, `moss_project_key` / `MOSS_PROJECT_KEY`, `fo_api_key` / `fo_api_ke`, `cartesia_api_key`, `apify_api_token` / `apify_api_key`, `brightdata_api_token`, `nessie_api_key` / `nessi_api_key`, `visa_cert_path`, `visa_key_path`. Environment names use the same words in upper case, and Bright Data also accepts `API_TOKEN`.

A missing key disables only that tool. This file does not record secret values. Outcomes of the v0.7.0 live probes are in `docs/sponsor-evidence.md`.
