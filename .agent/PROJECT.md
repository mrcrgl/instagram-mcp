# Project Rules (Repo‑specific)

This is the *only* place for repo‑specific rules; everything else should remain general.

---

## Overview
- **Purpose:** Read-only, multi-tenant MCP server that exposes Instagram professional accounts (own profile, media, comments, insights) and public data (hashtags, Business Discovery) to MCP clients such as Claude.
- **Primary constraints:** Read-only. Strict tenant isolation. No server-side state. Onboarding needs nothing but the server URL.
- **Explicit non‑goals:** Publishing, commenting, messaging, or any other write; personal Instagram accounts; scraping or any non-Meta data source; storing content or tokens on the server.

## Architecture summary
- **High‑level modules/layers:**
  - `domain`: identities, validated values (`ObjectId`, `Username`, `HashtagName`, `Cursor`, `MetricName`), content, insights, and pure analytics.
  - `instagram`: use cases (`InstagramService`) and the `InstagramStore` port.
  - `oauth`: stateless authorization server and the `IdentityProvider` port.
  - `mcp`: tool schemas and mapping.
  - `meta`: Graph API adapter implementing both ports.
  - `app`: composition root.
- **Dependency direction:** `mcp`, `oauth` → `instagram` → `domain`; `meta` → ports.
- **Forbidden dependencies:**
  - `domain` must not depend on anything else in this crate, on HTTP, or on Meta.
  - `instagram` and `oauth` must not depend on `meta`. Only `app` wires it in.
  - `mcp` tools must not call the Graph API or read tokens.

## Technology stack
- **Languages:** Rust
- **Frameworks/libraries:** `rmcp`, `axum`, `reqwest`, `chacha20poly1305`
- **Database/storage:** None. See `.agent/CONTEXT/DECISIONS.md`. The SQLx rules in `LANGUAGE_RULES.md` do not apply.
- **Observability:** `tracing` with structured fields

## Model boundaries & mapping
- **Domain models live in:** `src/domain/`
- **API DTOs live in:** `src/mcp/dto.rs`, `src/oauth/dto.rs`
- **Persistence (wire) models live in:** `src/meta/models.rs`
- **Mapping code lives in:** `src/meta/mapping.rs`, `src/mcp/dto.rs`
- **Mapping tests live in:** next to the mapping code

## Error handling conventions
- **Error types:** `thiserror` enums per boundary: `StoreError`, `ServiceError`, `ValidationError`, `AccountError`, `IdentityError`, `TokenError`, `FlowError`.
- **User‑facing error mapping:** Tool failures are tool results (`isError`), so the model can act on the message. Protocol errors are for server faults. Graph API error codes are classified once, in `meta::api::classify`.

## Testing & quality gates
- **Required test types:** Unit tests for domain, mapping, and protocol rules. Integration tests in `tests/` run the real app against a fake Graph API.
- **Tenant isolation:** New tools need a test proving they stay inside the tenant.
- **Security controls:** A test must fail when a control is removed. Check this by disabling the control.
- **Mandatory commands:** See `.agent/DOD.md`.

## Security & compliance
- **Secrets management:** Environment only. `Config`, `GraphApi`, `MetaIdentity`, and anything holding a token must not implement `Debug`. `Credential` has a redacting `Debug`.
- **Tenant source:** A `Session` is built only from a validated access token. The `account` argument only selects among the token's accounts.
- **Object access:** Media ids from tool arguments are checked against the tenant's accounts (`InstagramService::media`) before any further call.
- **Graph inputs:** Only the validated domain types may reach a Graph API path or field expression.
- **Logging:** Never log tokens, codes, sealed values, query strings, captions, comments, or Meta error messages. Tool failures are logged by `kind()` only.
- **New sealed values:** Add a `Purpose`. Never reuse one across meanings.
- **PII:** Only the app-scoped Meta user id and Instagram account ids and usernames, inside sealed tokens and logs.

## Workflow specifics
- **Default branch:** `main`
- **Merge strategy:** Squash

## Documentation expectations
- **Docs that must be updated for changes:** `README.md` for tools, configuration, data availability, or the Meta app setup.
- **ADR/decision log location:** `.agent/CONTEXT/DECISIONS.md`
- **What belongs in the decision log:** stable layer ownership, domain/API/persistence boundary rules, contract versioning rules, and cross-cutting runtime policies.
- **What does not belong in the decision log:** issue-slice refactors, extracted services, renamed files, file splits, test-module reorganizations, or cleanup passes unless they create a reusable repo-wide rule.
- **Where implementation history goes:** issue and PR updates by default.

## Language‑specific conventions
### Rust
- **Async traits:** Ports return `BoxFut`, because they are used as trait objects.
- **Lint configuration:** `cargo clippy --all-targets --all-features -- -D warnings`

---

## Repo quickstart
- **Test:** `cargo test`. No credentials needed.
- **Run:** See the configuration table in `README.md`.
- **Common pitfalls:** rmcp rejects unknown `Host` headers with 403, so `PUBLIC_BASE_URL` must match the public host. Meta renames and retires metrics often: defaults live in `src/domain/insights.rs`.
