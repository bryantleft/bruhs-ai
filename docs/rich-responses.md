# Rich AI responses

Bruhs displays images, video, audio, PDFs, and interactive HTML in assistant
replies on the web and in the desktop app. Markdown, tables, math,
Mermaid, and highlighted code keep their existing renderers.

Both built-in providers receive the response format and artifact-tool
instructions. Supported external provider adapters also expose the artifact
MCP tools when app tools are enabled.

## Images, playback, and metadata

Use normal Markdown for images and media links, or an explicit media block:

````markdown
```bruhs-media
{
  "kind": "video",
  "src": "output/demo.mp4",
  "title": "Dashboard walkthrough",
  "poster": "output/poster.png",
  "tracks": [
    {"src":"output/captions.vtt","label":"English","language":"en","kind":"subtitles"}
  ],
  "transcript": "A text transcript of the walkthrough."
}
```
````

`kind` is `image`, `video`, `audio`, or `pdf`. `src` is required. Other fields
are optional. Caption files use WebVTT. Track kinds are `subtitles`,
`captions`, `descriptions`, `chapters`, or `metadata`. Transcripts are supplied
text; these display features do not generate media or transcribe recordings.
PDF support and codecs depend on the installed browser engine and system
media components.

Nested images retain their table, list, and emphasis structure. External
sources require an explicit **Load** action, including external posters and
subtitle tracks. Playback never autoplays. Images offer zoom/fullscreen and
media can be downloaded. Local playback streams with HTTP Range support.

Local paths must resolve inside the thread's project/worktree or Bruhs
attachments. Canonicalization rejects traversal and escaping symlinks.
Files outside that scope must first be attached or copied into the project.
Project-less chats receive the concrete persistent media output directory.

Captured bytes live in private app data, not the system temporary directory.
Completed turns capture local media even when the thread is offscreen.
Before completion, immutable source bindings are sent to both clients and
written into the saved reply. A later turn may overwrite `output.png` without
changing an earlier reply. Legacy temporary attachments remain readable and
are copied into durable storage when used. Files already deleted before
capture cannot be recovered.

The normal page's content policy permits only local API media. Explicitly
loaded external media uses a separate controlled document. Mixed sources and
captions can use asset-specific signed URLs; the capability authorizes only
one immutable media file. It does not grant access to other files or APIs.
External downloads/proxy captures are POST operations initiated by user
controls, so an image URL cannot silently invoke the proxy. The resolver
rejects local, private, and reserved addresses on every connection and
redirect. External downloads retain the existing 50 MiB attachment budget.

## Persistent interactive widgets

Use a closed `bruhs-html` fence for self-contained HTML, CSS, JavaScript,
SVG, or canvas. A title after the language label is optional:

````markdown
```bruhs-html Counter
<button id="count" type="button">Count: 0</button>
<button id="use" type="button">Use in chat</button>
<script>
(async () => {
  await window.bruhs.ready;
  let count = window.bruhs.getState()?.count ?? 0;
  const button = document.querySelector('#count');
  const render = () => { button.textContent = `Count: ${count}`; };
  render();
  button.addEventListener('click', () => {
    count += 1;
    window.bruhs.setState({ count });
    render();
  });
  document.querySelector('#use').addEventListener('click', () => {
    window.bruhs.submit({ label: 'Use count', message: `Use a count of ${count}.` });
  });
})();
</script>
```
````

Form values, checks, selections, and scroll positions save automatically.
Use stable `id`, `name`, or `data-bruhs-key` attributes. Custom state such as
counters uses `bruhs.getState()` and `bruhs.setState(value)` after
`bruhs.ready`. `bruhs:restore` is also dispatched with restored custom state.
State is scoped to the saved response and its HTML fingerprint. Changing
the source starts a separate widget state. Widget state is bounded to a
256 KiB response-state budget.

Password/file controls and credential autocomplete fields are excluded from
automatic snapshots, including normal password-visibility toggles. Add
`data-bruhs-no-persist` to exclude another control. Custom state is supplied
by the widget author; avoid explicitly storing sensitive values there.

Widgets resize from their content and receive theme variables, including
`--bruhs-background`, `--bruhs-text`, `--bruhs-accent`, and `--bruhs-font`.
Prefer natural document height; give charts explicit dimensions rather than
using viewport-height layouts. Source, reload/reset, fullscreen, and HTML
save controls remain outside the widget. Mounted widgets keep their state
while text streams. Native virtualization flushes state before disposing an
offscreen view, and reopening a saved thread restores its widget values.

`bruhs.submit({label,message})` proposes a draft. The app displays the draft
for review and offers insertion into the composer. It never sends the
message, runs a command, or changes app records automatically. Normal agent
and domain permission checks still apply after the user chooses to send.

## Published HTML files and preview feedback

`publish_artifact(path, title?)` reads an HTML file in the tool's project or
Bruhs attachments, bundles its supported local assets, and stores an immutable
snapshot. Tool results contain a `bruhs-artifact` block with a published UUID.
Both interfaces can preview the tool result before the agent's final answer.

Local stylesheets, recursive CSS imports, fonts, classic scripts, images,
audio/video, and VTT tracks can be bundled. Source deletion does not destroy
the published artifact. Unresolved module imports/exports, runtime loaders,
remote dependencies, import maps, responsive `srcset`, nested frames, and
unsupported CSS imports fail with actionable errors. Prebundle such inputs
with the project's normal build tool; they are not silently omitted.

The preview is served by ID rather than putting a potentially large document
in a URL. `inspect_artifact(id)` returns actual client preview status and
runtime errors. Before a client renders it, inspection says **not yet
previewed**. It does not fabricate a headless screenshot or a successful
preview. The sandbox runs the bundled document after explicit rich syntax;
ordinary `html` code fences remain code examples. An incomplete rich fence
never executes, including after a stopped or failed turn.

## Portable export

**Download reply bundle** creates a ZIP containing `reply.md`, an offline
`index.html`, media files, standalone widget documents, and a manifest.
Sources are rewritten to bundled paths rather than depending on the original
project. Published artifact IDs are resolved into their bundled HTML.

Saved widget values and external media are separate opt-ins. By default,
exports omit entered widget values and retain external sources as references
without fetching them. The offline index uses links for unbundled external
media rather than loading it automatically. Opted-in widget values use the
latest client snapshot even if its background save is still pending.

## Isolation and storage

Assistant HTML runs in an opaque-origin `allow-scripts` iframe. It cannot
read the parent chat, local files, credentials, or general app APIs. Its
content policy blocks fetches, external resources, nested frames, plugins,
and form submissions. The trusted outer frame permits only `about:` child
frames, preventing widget code from navigating its frame to an external URL.
Camera, microphone, location, clipboard, and capture permissions are denied.

The narrow bridge validates its sender, protocol, and instance nonce. It
supports only state, size, errors, readiness, and reviewable chat drafts.
There is no command-execution or general app IPC surface in assistant HTML.
All other API routes retain their origin guard and framing restrictions.
Only asset-specific capabilities permit opaque-origin media reads.

Storage follows the app's private data directory. `BRUHS_ATTACHMENTS_DIR`
and `BRUHS_RICH_RESPONSE_DIR` can override the media and response storage
locations for isolated development/tests. Keep those locations persistent
and consistent between desktop, server, and provider helpers.

Native embeds are clipped to the transcript and hidden behind overlays and
inactive threads. They use WKWebView on macOS, Microsoft Edge WebView2 on
Windows, and WebKitGTK 4.1 on Linux. Windows needs the WebView2 Runtime.
Linux needs GTK 3, WebKitGTK 4.1, and an X11 display or XWayland; playback
formats also depend on the installed GStreamer plugins.

In a Wayland session with XWayland available, Bruhs selects its X11 backend
before starting threads so the browser can be attached inside the native
window. A pure Wayland session without XWayland keeps the app usable and
shows the missing display requirement beside Source and Open web app.
Missing browser runtimes also show an actionable error with web/source
fallback. Native hosts are destroyed before their parent window, and saved
widget state survives window closure and application shutdown.

The manual `Native desktop platforms` workflow builds Windows and Linux
and checks native lint and desktop tests. Its Windows GUI probe opens a real
WebView2 widget, checks sandbox isolation, clicks the counter through native
pointer input, closes the native window, and verifies its saved value after
reopening. The probe also rejects stale-window callbacks, teardown timeouts,
panics, and native handle errors after normal window closure. Windows clears
queued activation callbacks before destroying the native window.
Screenshots and fixture diagnostics are saved as workflow artifacts;
the private data/key directories are excluded. It does not
run automatically on pushes. A cross-build or CI build alone does not
establish live rendering on a particular OS or codec support.
