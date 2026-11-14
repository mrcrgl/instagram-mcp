# Decisions (ADR)

This file is the active architecture decision register.
Use it only for durable rules that should shape how future software is architected, written, and reviewed.

Keep an entry only when it establishes a long-lived constraint such as:
- boundary ownership
- dependency direction
- domain / API / persistence separation
- contract and versioning rules
- runtime or operational policies
- repository-wide coding standards

Do not add issue-scoped implementation notes, refactor mechanics, file moves, test reshaping, cleanup passes, or other work-log entries unless they create a reusable architectural rule future work must follow.

Format:
- Date (YYYY-MM-DD)
- Decision
- Why
- Consequences

---

## 2026-11-14: Instagram API with Facebook Login

- **Decision:** The server uses the Graph API at `graph.facebook.com` with Facebook Login for Business, not the Instagram Login variant at `graph.instagram.com`.
- **Why:** Only the Facebook Login variant offers hashtag search and Business Discovery, which are the main sources of public data.
- **Consequences:** Accounts must be professional and linked to a Facebook Page. The Meta app needs Page permissions and the Instagram Public Content Access feature.

## 2026-11-14: No server-side state; the Meta token travels sealed

- **Decision:** The server has no database. Client ids, codes, access tokens, and refresh tokens are sealed values. Access and refresh tokens embed the user's long-lived Meta token, because Meta has no app credential that can read a user's data.
- **Why:** Open client registration on a public endpoint must not be able to fill a store, and holding no tokens at rest removes a whole class of breach. Any replica can serve any request.
- **Consequences:** A refresh token is not single-use: it is bound to its client and expires with the Meta token (at most 60 days). Single sessions cannot be revoked server-side; removing the app in Facebook revokes all of them. Do not add storage to work around this without revisiting this decision.

## 2026-11-14: The tenant comes only from the access token

- **Decision:** Every use case takes a `Session` built only by `OAuthService::authenticate`. The `account` argument selects among the token's accounts and nothing else. Media ids from arguments must belong to one of them.
- **Why:** Isolation has to hold by construction, not by trusting Meta to refuse every cross-account read.
- **Consequences:** Tools cannot read another tenant's media even where Meta would allow it. Requests run without MCP sessions.

## 2026-11-14: Read-only

- **Decision:** The server exposes no tool that writes to Instagram, and requests no write permission except `instagram_manage_comments`, which Meta requires for reading comments.
- **Why:** The purpose is analysis. Write access would turn untrusted content (captions, comments) into a lever for prompt injection with side effects.
- **Consequences:** Adding a write tool requires revisiting this decision, a separate scope, and its own consent wording.

## 2026-11-14: Consent before Facebook

- **Decision:** The server shows its own consent page, bound to the browser by a `__Host-` cookie, before redirecting to Facebook.
- **Why:** Registration is open, and Facebook does not ask again once a user has authorized the app. Without this, any registered client could obtain a user's access by sending them a link.
- **Consequences:** Users see one more step. It must not be skipped or remembered per client.

## 2026-11-14: Only validated types reach the Graph API

- **Decision:** Path segments and field expressions are built only from `ObjectId`, `Username`, `HashtagName`, `Cursor`, and `MetricName`, or from tenant account ids.
- **Why:** Business Discovery interpolates the username and cursor into a field expression, and ids become path segments. Type constructors make injection impossible by construction.
- **Consequences:** New inputs that reach a request need a domain type with a restrictive constructor and tests for hostile values.
