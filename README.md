# instagram-mcp

A read-only [MCP](https://modelcontextprotocol.io) server for Instagram
professional accounts. It gives Claude and other MCP clients your own
profile, media, comments, and insights, plus public hashtag data and
benchmarks of other business accounts.

The server stores nothing. It has no database, and your Meta token travels
only inside the encrypted tokens your MCP client holds.

## What data Instagram makes available

Meta's [Instagram Platform](https://developers.facebook.com/docs/instagram-platform)
is the only sanctioned API. This server uses its **Facebook Login** variant
because that variant has hashtag search and Business Discovery. The
Instagram Login variant has neither.

| Data | Available | Notes |
|---|---|---|
| Your profile: bio, followers, following, media count | Yes | `account_profile` |
| Your posts, reels, and carousels with likes and comments | Yes | `media_list`, `media_get` |
| Per-post insights: views, reach, saves, shares, watch time, skip rate | Yes | `media_insights`. Story insights only within 24 hours |
| Account insights: views, reach, engagement, follows, link taps | Yes | `account_insights`. At most 30 days per request, up to 2 years back |
| Audience by age, gender, country, and city | Yes | `audience_demographics`. Needs 100+ followers |
| Comments on your media | Yes | `comments_list` |
| Your live stories | Yes | `stories_list` |
| Media you are tagged in | Yes | `tagged_media` |
| Public posts for a hashtag, top or last 24 hours | Yes, limited | `hashtag_media`, `hashtag_report`. **30 distinct hashtags per account per 7 days** |
| Another business or creator account's profile and recent posts | Yes | `business_profile`, `business_report`. Likes, comments, and reel views only |
| Keyword search in captions | Partly | `caption_contains` filters your media or hashtag media. There is no global keyword search |
| Personal (non-professional) accounts | No | Not even public ones |
| Home feed, explore, trending topics or sounds | No | Not exposed by any API |
| Reach or saves of other people's posts | No | Only likes, comments, and reel views |
| Follower lists, who liked a post | No | |
| Hashtag volume (how many posts use it) | No | `hashtag_report` estimates from a sample instead |

## Tools

All tools are read-only. `account` picks one of your connected accounts by
username and can be omitted when only one is connected.

| Tool | Purpose |
|---|---|
| `accounts_list` | Connected accounts with follower and media counts |
| `account_profile` | Profile of a connected account |
| `media_list` | Your media, newest first. Filter by date or `caption_contains` |
| `media_get` | One media item |
| `media_insights` | Per-media metrics, with sensible defaults per format |
| `comments_list` | Comments on your media |
| `stories_list` | Stories live right now |
| `tagged_media` | Media you are tagged in |
| `account_insights` | Account metrics over a date range, as totals, a time series, or broken down by format |
| `audience_demographics` | Followers or engaged audience by age, gender, country, or city |
| `account_report` | Engagement, posting rhythm, best formats and times, hashtags, and top posts over your recent media |
| `hashtag_media` | Top or recent public posts for a hashtag |
| `hashtag_report` | Typical engagement, formats, timing, and related hashtags for a hashtag |
| `hashtags_searched` | Hashtags already looked up this week. Reusing them costs nothing |
| `business_profile` | Another business account's profile and recent posts |
| `business_report` | The same analysis as `account_report`, for a competitor |

Engagement in reports is likes plus comments, the only figures Meta gives for
content you don't own. The engagement rate is the mean per post divided by
followers. Times are UTC.

## Connect

Requirements on your side:

- An Instagram **business or creator** account, linked to a Facebook Page.
- A Facebook user who can manage that Page. While the Meta app is in
  Development mode, that user also needs a role on the app (see below).

**Claude (web and desktop):** *Settings → Connectors → Add custom connector*,
then enter `https://<host>/mcp`.

**Claude Code:**

```bash
claude mcp add --transport http instagram https://<host>/mcp
```

Your client registers itself (OAuth dynamic client registration), so there
is no client id to copy. A browser opens. Confirm the client, sign in with
Facebook, and select the Page and Instagram account to share.

To disconnect, remove the app under Facebook *Settings → Business
integrations*. Access stops immediately.

## Self-hosting

### 1. Create the Meta app

On [developers.facebook.com](https://developers.facebook.com/apps):

1. Create an app of type **Business**, and add **Facebook Login for Business**.
2. Under *Facebook Login for Business → Settings*, set
   **Valid OAuth Redirect URIs** to `https://<host>/oauth/meta/callback`.
   Keep *Enforce HTTPS* and *Use Strict Mode for redirect URIs* on.
3. Under *App settings → Advanced*, turn on **Require App Secret**. The
   server signs every call with `appsecret_proof`.
4. Request these permissions (*App Review → Permissions and Features*):

   | Permission | Why |
   |---|---|
   | `instagram_basic` | Profile and media |
   | `instagram_manage_insights` | Insights, and Business Discovery |
   | `instagram_manage_comments` | Reading comments. Meta has no read-only scope for them |
   | `pages_show_list`, `pages_read_engagement` | Finding the Instagram account through its Page |
   | `business_management` | Pages owned by a Business portfolio |
   | Feature **Instagram Public Content Access** | Hashtag search |

5. Optional: under *Facebook Login for Business → Configurations*, create a
   configuration with a **User access token** and the permissions above. Set
   its id as `META_LOGIN_CONFIG_ID`. Without one, the server requests the scopes
   directly.

In **Development mode**, only people with a role on the app (admin, developer,
or tester) can sign in, and standard access is enough for their own accounts.
That fits a private deployment. Letting anyone else connect needs Advanced
Access through App Review and Business Verification.

### 2. Configure

| Variable | Required | Meaning |
|---|---|---|
| `PUBLIC_BASE_URL` | yes | Public origin, e.g. `https://instagram-mcp.example.com`. Must be `https` outside localhost |
| `TOKEN_KEYS` | yes | Comma-separated token keys, newest first |
| `META_APP_ID` | yes | The app's numeric id |
| `META_APP_SECRET` | yes | The app secret |
| `META_LOGIN_CONFIG_ID` | no | Facebook Login for Business configuration id |
| `META_GRAPH_VERSION` | no | Default `v26.0`. Pin it, and move it forward deliberately |
| `META_GRAPH_URL` / `META_WEB_URL` | no | Default to graph.facebook.com and www.facebook.com |
| `PORT` / `RUST_LOG` | no | `10080` / `info` |

Generate a token key:

```bash
openssl rand 32 | basenc --base64url --wrap=0 | tr -d =
```

Every replica needs the same `TOKEN_KEYS`. To rotate, put a new key first and
keep the old one for 60 days, which is how long refresh tokens live. Removing a
key signs out everyone whose tokens it sealed.

### 3. Run

```bash
docker build -f _build/Dockerfile -t instagram-mcp .
docker run -p 10080:10080 --env-file .env instagram-mcp
```

The server needs TLS in front of it, and the proxy must forward the original
`Host` header. `GET /healthz` is the probe endpoint. It keeps no state, so run
as many replicas as you like, without sticky sessions.

On Kubernetes, use the chart in `_build/helm/instagram-mcp`. Images are
published to `ghcr.io/mrcrgl/instagram-mcp` as `sha-<commit>`. The chart reads
its secrets from an existing Secret, so they never pass through Helm:

```bash
kubectl create namespace instagram-mcp
read -rs META_APP_SECRET
kubectl -n instagram-mcp create secret generic instagram-mcp \
  --from-literal=TOKEN_KEYS="$(openssl rand 32 | basenc --base64url --wrap=0 | tr -d =)" \
  --from-literal=META_APP_SECRET="$META_APP_SECRET"

helm upgrade --install --atomic --wait -n instagram-mcp \
  -f _build/helm/values.production.yaml --set image.tag=sha-1a2b3c4 \
  instagram-mcp _build/helm/instagram-mcp
```

`_build/helm/values.production.yaml` has the values for the production
deployment. Fill in `meta.appId` first.

## Security model

- **Read-only.** No tool writes to Instagram. `instagram_manage_comments` is
  the only granted scope that could write, and no code path uses it.
- **Tenancy.** A session comes only from a validated access token. Tools can
  act only as the Instagram accounts that token's Meta user shared. Media ids
  passed to tools must belong to one of those accounts, which the server checks
  before Meta is asked for insights or comments.
- **Inputs.** Ids, usernames, hashtags, metric names, and cursors are parsed
  into types before any request is built. A username or cursor cannot change
  a Business Discovery field expression, and an id cannot address another
  object path.
- **Tokens.** Client ids, codes, and tokens are encrypted with
  XChaCha20-Poly1305 and bound to their purpose. The Meta token inside them is
  never visible to the client. Access tokens last one hour, are bound to this
  server's `/mcp` resource, and never outlive the Meta token. Meta tokens go in
  the `Authorization` header, never in URLs, with an `appsecret_proof` on every
  call.
- **Consent.** Registration is open, so a consent page names where tokens will
  go before anything reaches Facebook. A browser-bound cookie stops a link from
  being completed in someone else's browser.
- **Untrusted content.** Captions, comments, and bios are written by other
  people. The server's instructions tell the model to treat them as data, not
  as instructions.
- **Logs.** No tokens, codes, query strings, captions, comments, or Meta error
  text. Tool failures are logged by kind.

Because there is no server-side state:

- An authorization code can be replayed for its 60-second lifetime. PKCE still
  makes a stolen code useless without the verifier.
- A refresh token is not single-use. It is bound to its client and expires with
  the Meta token, after at most 60 days. Removing the app in Facebook settings
  ends every session at once.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

The tests run the real application against a fake Graph API, so they need no
credentials. See [`AGENTS.md`](AGENTS.md) for the working agreements.

## License

[MIT](LICENSE)
