# Live OpenAI test through goose

This is **not** the direct-HTTP `openai-live-smoke.py` script. It invokes a freshly built `goose run` for every case, using the real CLI/provider, streaming Responses, a temporary isolated goose configuration, and a developer-shell tool round trip. Each case is billed. Nothing is pushed or written to your normal goose configuration; no API key is included in the script.

From the repository root:

```bash
source bin/activate-hermit
cargo build -p goose-cli --bin goose
# Export OPENAI_API_KEY in your shell using your normal secret manager.
python3 documentation/model-metadata-investigation/openai-goose-live.py
```

By default it makes four runs on `gpt-5.6-terra`: streaming text, `GOOSE_THINKING_EFFORT=low`, `=medium`, and a developer-shell call (`printf GOOSE_OPENAI_TOOL_OK`) with a response after tool replay. It checks CLI stream events, tool request and result, final assistant report, and completed run; prints only usage/cost, not the prompt or key. The script uses `GOOSE_PATH_ROOT` to isolate configuration/state, `--no-profile`, `--no-session`, `GOOSE_MODE=auto`, a 3-turn bound per run, and the implicit native OpenAI Responses route. It overwrites `OPENAI_HOST` and clears `OPENAI_BASE_URL`/`OPENAI_BASE_PATH` in child processes. A model may make multiple billed API calls per run.

To select a model or goose thinking levels:

```bash
OPENAI_SMOKE_MODEL=gpt-6-astra OPENAI_SMOKE_EFFORTS=low,medium \
  python3 documentation/model-metadata-investigation/openai-goose-live.py
```

`OPENAI_SMOKE_EFFORTS` accepts goose levels `off,low,medium,high,max`. `off` maps to `none` **only when the catalog lists it**; otherwise it maps to `low`. `max` maps to the highest supported catalog level and may be substantially more expensive. These tests exercise the goose mapping and API acceptance, not a proof of how much reasoning the model performed. Set `GOOSE_STATE_MACHINE=1` to exercise the alternate agent loop in another run. Use `GOOSE_SMOKE_BINARY=/absolute/path/to/goose` to test another build. The shell tool is explicitly limited to a harmless printf in the prompt, but the developer extension executes model-requested shell commands; run only in an environment where that risk is acceptable.

The script does not run automatically during unit tests, and no billed requests were made while developing it. Failures stop immediately and may print the final portion of goose output/stderr for diagnosis; check before sharing it publicly.
