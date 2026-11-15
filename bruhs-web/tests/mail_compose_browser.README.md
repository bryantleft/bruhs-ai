# Email compose browser checks

Use a fresh DEFAULT production capture that includes the current compose files
and adopted HTTP error reader. The driver refuses stale sources or missing WASM,
records every asset SHA256/mtime and source hash, and rechecks them after the run.
It uses an existing Playwright and Chrome installation, without installing,
building, changing app source or writing Git state.

    BRUHS_COMPOSE_BUILD_RECEIPT=/absolute/build.json \
    BRUHS_COMPOSE_QA_OUTPUT=/absolute/new/evidence \
    node bruhs-web/tests/mail_compose_browser.mjs /absolute/fresh/dist

The receipt is optional for development; acceptance should supply the successful
capture's receipt. The fixture tests Gmail and Outlook at 1280×800, 640×480 and
390×844 with coarse pointer and reduced motion. The app is actual compiled WASM,
using real IndexedDB, Blob, FormData, input and review controls. API responses are
synthetic and every unrelated request/mutation is rejected.

Checks include incomplete recipient text, local/server save, exact body after
reload, current connection generation, native multipart file integrity, a lost
upload response with the same durable id/bytes, injected transaction abort with
retained input and explicit retry, full inert review, bounded feedback, reachable
footer/touch targets, explicit Send, durable operation id, uncertain status,
reload without a second send POST, and acknowledgement recovery without replay.
Reports retain requests, synthetic drafts/statuses, expected failures, checks,
source/asset pins and actual screenshots. The driver never supplies OAuth,
credentials or access to a real provider and never sends real email. Accepted
synthetic receipts do not establish provider acceptance or recipient delivery.

The separate connected_mail_browser.mjs remains the strict read/sync/download and
stale GET cancellation regression driver. Do not weaken or replace it to obtain a
compose pass. The server integration suite separately checks actual production
HTTP routes, native ServerClient, broker/cache/MIME/provider transport fixtures.

Geometry also sweeps320/639/640/641/768/1024/1920px as applicable to each
context, retaining the actual touch/coarse-pointer setting. For targeted reruns,
set `BRUHS_COMPOSE_QA_PROVIDERS` and `BRUHS_COMPOSE_QA_CELLS` to comma-separated
provider/viewport names. `BRUHS_COMPOSE_QA_SLOW3G=1` applies CDP400ms RTT and
400kbps down/up, cache off, after each normal app load. It measures warm-app
interaction resilience only; full WASM loads/reloads stay unthrottled and no
cold-load, latency or performance acceptance is implied.
