# Service connections

Google Workspace, Microsoft 365, and GitHub share the Connect flow in
Settings → Credentials and in agent account-access requests. Each provider has
one connected account per device. Disconnect before switching accounts or OAuth
registrations. These accounts are separate from Calendar sync and saved API keys.

## What users see

Connect opens the normal browser on desktop, or a new tab in the web app's
current browser profile. The provider can reuse that profile's SSO session.
Consent, account selection, MFA, and organization policy can still require user
interaction. Bruhs does not read browser cookies or collect SSO passwords.

The account, requested access, granted scopes, and connection status are visible.
Reconnect uses the same flow. Google may require renewed consent after local
credentials are lost; it does not force a fresh password login. Expired tokens
refresh privately when the provider supplied a refresh token. Revoked grants
require sign-in again.

Connecting never approves an operation. The agent's `request_connection` tool
opens a private review showing the exact command, working directory, service,
account, and token variable. Approve operation passes only an access token to
that command. Command output is discarded; the model receives only status and
exit code. Refresh tokens and client secrets never enter provider events.

Disconnect deletes local tokens and invalidates old operation references and
pending sign-ins. It leaves the normal browser session and the provider's app
grant intact. Provider-wide revocation can affect other integrations, including
Google Calendar. Users can revoke the app separately in their provider's account
settings. Commands already started may retain their process-local access token.

Browser-tab automation is a separate planned integration. OAuth sign-in grants
neither tab control nor access to other signed-in websites.

## Operator setup

Register a client for each service before enabling its Connect control. Set the
following variables in the environment of the Bruhs server and/or desktop process
that owns the connection. Web sign-in is owned by the local companion server;
desktop sign-in is owned by the desktop process. If using both, configure the
same registrations in both processes. Restart them after changing registration.
Do not commit client secrets to the repository or paste them into chat.

The web UI and companion server must run on the same machine as the user's
browser. Remote servers and cross-device sign-in are not supported by this local
loopback flow.

| Service | Registration | Environment |
|---|---|---|
| Google Workspace | Google OAuth Desktop app; enable Calendar API and configure consent/test users | `BRUHS_CONNECTION_GOOGLE_CLIENT_ID`; optional `BRUHS_CONNECTION_GOOGLE_CLIENT_SECRET` if supplied by the registration |
| Microsoft 365 | Entra Mobile and desktop application; allow public-client authorization and the configured tenant audience | `BRUHS_CONNECTION_MICROSOFT_CLIENT_ID`; optional `BRUHS_CONNECTION_MICROSOFT_TENANT` (default `organizations`) |
| GitHub | OAuth app with a loopback callback | `BRUHS_CONNECTION_GITHUB_CLIENT_ID` and `BRUHS_CONNECTION_GITHUB_CLIENT_SECRET` |

The native broker listens on a random IPv4 loopback port and uses
`http://127.0.0.1:<port>/` as the redirect URI. It owns the callback in both web
and desktop modes; no callback is added to the companion API's origin exemptions.
Register the loopback root URI using the provider's native-app rules. Entra's
HTTP `127.0.0.1` redirect needs manifest configuration. GitHub permits the
loopback port to vary; use the same root path in its callback registration.

Use separate registrations from the existing Calendar sync integration. Google
OAuth apps in Testing and enterprise consent policies can restrict token lifetime
or access. GitHub requires a client secret for this code exchange even with PKCE;
each installation supplies its own registration. No shared confidential secret
is embedded in the application.

Initial permissions are deliberately limited:

- Google: `openid`, `email`, `calendar.events.readonly` for Calendar events.
- Microsoft: `openid`, `profile`, `email`, `offline_access`, and Graph
  `Calendars.ReadBasic` for basic calendar details.
- GitHub: `read:user`, `read:org`, and `offline_access` for profile, organization
  membership, and renewable access. This does not grant private repository access.

These are service access tokens, not universal credentials for other resources.
Adding write permissions or another API requires a separate scope decision and
user consent.

## Storage and lifecycle

Tokens use the OS keychain service `bruhs-ai-connections`. The app data directory's
`connections/<provider>.json` contains metadata and generation fences, never token
values. Listings read metadata without unlocking the keychain. A cross-process
lease serializes login commits, refresh rotation, and disconnection. Blocking
keychain work retains that lease even when its caller disconnects or is canceled.

Failed writes remain visible for recovery. A durable disconnect fence prevents
old token bundles and late callbacks from reviving an account. Reconnection
checks the registered client and the provider's stable account identity before
preserving an existing refresh token. Every successful sign-in issues a new
operation reference.

The broker removes all service client-secret environment variables from agent
provider processes, review processes, and approved credential commands. The
existing credential boundary still applies: an approved command can transmit a
token it receives. This feature is not a sandbox against a malicious approved
command or a compromised local application.

## Verification

`cargo test -p bruhs-core --lib` covers synthetic OAuth providers, token privacy,
rotation, disconnect fences, metadata-only recovery, canceled storage I/O, and
per-operation approval. No test reads a user's keychain.

The UI acceptance fixture uses the real WASM application and synthetic local
services. It verifies all three Connect flows, error/retry, reordered responses
after cancellation, desktop/mobile layout, and the absence of automatic command
approval. It saves screenshots in a temporary directory:

```sh
(cd bruhs-web && env -u NO_COLOR BRUHS_API_URL='' trunk build --dist /tmp/bruhs-connections-dist)
BRUHS_PLAYWRIGHT_PATH=/path/to/playwright \
  node scripts/verify-service-connections.playwright.mjs /tmp/bruhs-connections-dist
```

Live SSO session reuse, tenant policies, and OS keychain prompts require a
separate test with registered clients and user-authorized accounts. Synthetic
tests cannot establish those live-provider outcomes.

References: [Google desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app),
[Microsoft authorization code flow](https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow),
[Microsoft redirect rules](https://learn.microsoft.com/en-us/entra/identity-platform/reply-url),
[GitHub OAuth](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/authorizing-oauth-apps).
