# goose-bharatgen

BharatGen fork of Goose, pinned to upstream **v1.52.0** (Apache-2.0; license and notices unchanged).
Used with the Manch LiteLLM gateway (Bharatgen-Tech/mancha-code).

## Branches
- `prod`: exactly upstream v1.52.0. Nothing else.
- `dev`: v1.52.0 plus BharatGen changes (`init-config.yaml`, this file, later branding).

## Rules
- Keep changes small and isolated (config files, a few branding files) so upgrading is a merge, not a rewrite.
- Upgrade by merging a newer upstream release tag into `dev`, test against the gateway, then fast-forward `prod`.
- Do not use Goose trademarks in a way that implies official endorsement (Apache-2.0 / CUSTOM_DISTROS.md).

## Defaults
Provider `litellm`, model `bharatgen-param`, telemetry off. The gateway URL and a per-user key come from the environment.

## White-label TODO (MANCH-20)
App name and config dir: `crates/goose/src/config/paths.rs`. Desktop name/icons: `ui/desktop/`. Prompts: `crates/goose/src/prompts/`.
Telemetry endpoint: `crates/goose/src/posthog.rs`.
