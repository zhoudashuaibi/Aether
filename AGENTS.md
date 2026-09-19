# Aether Agent Guide

## Scope and layout
- Aether is a self-hosted multi-tenant AI API gateway for Claude, OpenAI, Gemini, and CLI clients. Read `README.md` for local setup and deployment assumptions.
- Rust workspace (Rust **1.95.0**) is the backend: `apps/aether-gateway` is the primary service and default workspace member; `apps/aether-tunnel` is the optional proxy-node binary. Domain crates live in `crates/`.
- `frontend/` is a Vue 3 + TypeScript + Vite admin UI (`components/`, `views/`, `stores/`, `router/`).
- `crates/aether-data/contracts` owns cross-crate persistence DTOs, traits, and errors. `crates/aether-data/runtime` owns concrete data composition, migrations, backfills, and import/export; read its `README.md` before changing data-layer code.

## Commands
- Set up local configuration with `cp .env.example .env`; do not commit `.env` or generated keys.
- Full local stack: `make dev`; backend only: `make dev-backend`; frontend only: `make dev-frontend`. These use the gateway health endpoint and may start local Postgres/Redis through Docker.
- Database maintenance: `make migration` and `make backfill` (require `.env`). For schema changes, run `bash crates/aether-data/runtime/schema/compose_schema.sh generate`, `compose`, and `check`.
- Rust verification: `cargo fmt --all --check`; `cargo clippy -p aether-gateway --lib --bins --examples -- -D warnings`; focused tests use `cargo test -p <crate>` (CI uses `cargo nextest run` where available).
- Frontend (from `frontend/`): `npm run type-check`, `npm run lint` (this runs ESLint **with `--fix`**), `npm run test:run`, and `npm run build:with-typecheck`.

## Boundaries and conventions
- Preserve the contracts → driver adapters → repositories → backend composition → maintenance layering documented in `crates/aether-data/runtime/README.md`. Keep cross-crate types in `aether-data-contracts`; do not put domain queries in low-level pool modules or driver selection in repository implementations.
- Database support is PostgreSQL only after the September 2026 upstream sync. Keep PostgreSQL SQL in its adapter and test the deployed driver feature. For schema changes, edit `crates/aether-data/runtime/schema/logical/*.toml` or the documented driver fragments—not `schema/generated/**`—then run the schema compose/check commands from that README.
- The gateway uses PostgreSQL; the compatibility feature `all-drivers` now selects PostgreSQL only. SQLite/MySQL deployments need a separate data migration before upgrading.
- Keep `aether-ai-formats` limited to parsing, emitting, and conversion; transport policy belongs outside it. Preserve parsed unknown JSON fields for same-format requests, and fail closed rather than silently dropping unaudited cross-format fields.
- Match nearby Rust style and run rustfmt. Rust CI treats Clippy warnings as errors.
- Frontend components use Vue `<script setup>` and PascalCase component tags. Unused TypeScript args/variables must start with `_`; avoid `any`. `console` is forbidden in production except in `main.ts` and `logger.ts`.

## Release and CI (fork specifics)
- This fork (`zhoudashuaibi/Aether`) syncs from upstream `fawney19/Aether` and publishes its own releases: tag `v0.1.N` on `main` triggers `.github/workflows/release.yml`, which pushes the Docker image to `ghcr.io/zhoudashuaibi/aether`. Keep the fork's GHCR branding and `GHCR_TOKEN` wiring intact when merging upstream.
- **macOS release builds are not needed.** Deployment is Linux Docker only. When touching `release.yml` (e.g. after an upstream merge), remove the `macos-amd64`/`macos-arm64` matrix entries and the mac tarball packaging instead of keeping them: the `macos-15-intel` runner takes ~50 minutes and gates the whole release. Do not wait on or fix macOS-only build issues.
- Do not build Rust locally before pushing (disk constraints); rely on GitHub Actions (Rust CI on `main`, Release on tags) and iterate there. `cargo fmt --all --check` and `cargo metadata --locked` are safe locally.

## Read before sensitive changes
- API format/compatibility work: `docs/api/format-passthrough-contract.md`, `docs/api/format-conversion-audit.md`, and `docs/api/provider-interface-definitions.md`.
- Redis/runtime coordination: `docs/operations/redis-runtime-runbook.md`.
- Data architecture or migrations: `crates/aether-data/runtime/README.md`.
