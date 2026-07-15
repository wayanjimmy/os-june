---
status: accepted
date: 2026-07-15
---

# Notion connector uses user-supplied local tokens, not REST OAuth

## Context

June's connector promise is local mode: provider credentials stay on the Mac,
provider calls originate from the Mac, and OpenSoftware infrastructure holds no
credential that can read the user's connected account. ADR 0016 records that
shape for Google, where a native-app OAuth flow with PKCE can mint the user
refresh token directly to the device.

The Notion REST public OAuth surface is different. Public connections require a
confidential client secret for token exchange, token refresh, and token
revocation. The token endpoints use HTTP Basic authentication with
`client_id:client_secret`; the current Notion docs and token references do not
provide a REST PKCE or public-client alternative. Notion's token responses also
do not return `expires_in`.

Notion's hosted MCP service is a separate surface that supports OAuth with PKCE
and dynamic client registration, but using it would route Notion tool requests
through Notion's hosted MCP service rather than June's on-device REST client. A
June API broker could hold the REST client secret, but then OpenSoftware would
transiently receive or refresh Notion credentials. Embedding the client secret
in the desktop binary is not a real secret.

## Decision

June's Notion connector v1 will not use Notion REST public OAuth. It will use a
local token-paste connection flow:

- The user creates or selects a Notion internal connection or personal access
  token in Notion, copies the token, and pastes it into June.
- June stores the token only in the macOS Keychain, under a Notion-specific
  service (`co.opensoftware.june.notion`; debug builds use
  `co.opensoftware.june-dev.notion`). No Notion token material is stored in
  SQLite, logs, issue reports, Hermes workspaces, or MCP server environment.
- Notion REST calls originate from the app's Rust provider proxy and go directly
  from the device to Notion. June API is not in the Notion connector data path.
- The Notion MCP server processes receive only June's scoped loopback connector
  token and connector account id; they never receive the Notion token.
- Initial Notion connection is read-only in June's product flow. Users are
  instructed to configure a read-content internal connection and share selected
  pages with it in Notion. A personal access token is broader: it acts as the
  creating user and follows that user's Notion permissions, bounded by token
  capabilities rather than a provider-side selected-page sharing boundary. June
  presents PATs as an advanced fallback and still requires June-selected roots
  for every read or write.
- Insert/update capabilities require an explicit later capability update in
  Notion followed by June revalidation; a replacement token or reconnect is
  required only if Notion requires it. June still structurally gates every write
  through the Rust approval and action-journal path before any write route
  ships.
- Disconnect deletes the local Keychain token and local connector metadata. June
  cannot perform Notion's REST OAuth revocation for token-paste connections,
  because revocation applies to OAuth public connections and requires the
  connection's client secret. The disconnect UI must tell the user how to remove
  the internal connection/PAT in Notion if they want server-side invalidation.
- June pins `Notion-Version: 2026-03-11` on every Notion REST request. Before
  changing the pin, the implementation must document the API-version upgrade
  procedure, including Notion changelog review, supported block/property matrix
  updates, compatibility tests, and a live workspace smoke test.

This decision preserves local credential custody and the ADR 0016 local-mode
privacy claim for Notion at the cost of a less polished onboarding flow. Notion
hosted MCP remains a future option, but adopting it requires a new ADR and an
updated threat model because the request path and trust boundary differ.

## Consequences

- The Notion v1 onboarding is higher friction than OAuth: users must create or
  choose a Notion token. Internal connection users manually share pages or data
  sources with that connection in Notion; PAT users rely on the user's existing
  Notion permissions, so June's selected roots become the narrower boundary.
- Notion's OAuth page picker is not part of the v1 REST path. June must provide
  its own post-connect resource picker and treat Notion access as the maximum
  graph, not the selected graph. This is especially important for PATs, which
  may expose a broader user-permission graph than an internal connection.
- Marketplace-style Notion public OAuth distribution is deferred. The v1
  connector is a local-custody feature for users willing to supply their own
  token.
- June can honestly say OpenSoftware does not hold a Notion credential and is
  not in the Notion connector data path. The claim still does not cover model
  inference: Notion content included in a prompt follows the user's selected
  model path, as with other connectors.
- Write support must not assume OAuth scopes. Notion capabilities are attached
  to the connection or token, so June's enforcement point is the Rust proxy,
  selected-resource boundary, approval surface, and action journal.
- Lost or invalid tokens enter a reconnect state. For token-paste connections,
  reconnect means the user pastes a replacement token from Notion.

## Alternatives considered

### Notion REST public OAuth with an embedded secret

Rejected. A desktop binary cannot keep a Notion public-connection client secret
confidential. Anyone who extracts it could exchange, refresh, or revoke grants
for that public connection.

### June API confidential OAuth broker

Rejected for v1. It would improve onboarding and preserve Notion's OAuth page
picker, but OpenSoftware infrastructure would receive authorization codes and
refresh tokens or hold a credential capable of refreshing them. That violates
local mode's credential-custody claim and would require a different privacy
claim and threat model.

### Notion hosted MCP with PKCE

Deferred. This avoids a June-held REST client secret and may preserve a smoother
OAuth flow, but it changes the provider request path: June would call Notion's
hosted MCP service rather than a local REST client. That is a legitimate future
architecture, not a drop-in implementation detail, and needs its own ADR and
threat model update.

### User-supplied personal access token only

Allowed but not preferred. PATs can preserve local custody, but they act as the
user and may be broader than an internal connection. The product flow should
prefer internal connection tokens where possible and present PATs as an advanced
fallback.
