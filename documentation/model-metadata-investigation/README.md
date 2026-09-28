# Model-release maintenance audit (2026-09-28)

Starting point: `4dea9b483` adds a live models.dev canonical catalog with bundled fallback. This audit prioritizes OpenAI, Anthropic, and the two Databricks providers. It records findings only; no runtime behavior was changed. Paths below are repository-relative and line numbers refer to that starting commit.

- [OpenAI](openai.md)
- [Anthropic](anthropic.md)
- [Databricks](databricks.md)

## Cross-cutting observations

The canonical catalog supplies capabilities, not necessarily the **wire protocol** accepted by a deployment or gateway. Discovery of names (`/models`, serving-endpoints, gateway catalogs) likewise does not prove endpoint compatibility. Preserve configured paths and per-endpoint API-type metadata as authoritative; do not replace routing rules with catalog `reasoning` alone. For aliases, look up resolved upstream model where available and retain an explicit fallback. Cache, effort levels, and parameter support are different questions from whether a model reasons.

The relevant shared logic is in `crates/goose-provider-types/src/`: `model.rs:288-341` (reasoning fallback by name), `formats/openai.rs:1722-1788,1814-1938` (wire parameters and model-family/effort heuristics), `formats/openai_responses.rs:608-760` (GPT-5.6 mode and reasoning parameters), `formats/databricks.rs:505-625` (Claude/OpenAI formatting), `cache_semantics.rs:23-48` (name-dependent cache strategy), and `canonical/models_dev.rs:24-44` (exact-ID Anthropic thinking-mode overrides). These are not in `goose-providers` but are exercised by it. The latest commit makes catalog records refreshable, but does **not** eliminate these rules.

## Suggested sequence for follow-up changes

1. Separate provider/deployment route selection from reasoning capability; specify precedence of explicit configuration, endpoint API types, canonical capabilities, and conservative fallback per provider. Cover unknown and aliased names.
2. Move safe *capability* decisions to the live catalog/configuration where fields exist (reasoning, temperature, thinking mode); document what models.dev cannot represent (supported effort levels, transport endpoint, cache dialect, exceptional model parameters).
3. For provider-specific wire exceptions, prefer endpoint capabilities or scoped configuration over adding version strings. Avoid probing inference endpoints on every request without a cache/error policy and user consent for side effects.
4. Keep a deliberate default model and offline fallback; measure whether each known-model list is still needed for discovery/UI before removing it.

No build/tests were run: this is documentation-only investigation.
