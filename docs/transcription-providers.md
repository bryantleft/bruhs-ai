# Transcription providers

Settings → Transcription on web and desktop controls the final transcript for
the next recording. Changes during a recording do not change that recording's
provider, model, or endpoint. Manual and automatic recordings use the same
preferences.

## On-device Whisper

This is the default and needs no provider credentials. Choose Tiny English,
Base English, Small English, or Large v3 Turbo for the final transcript.
Existing saved model selections are preserved.

Live previews always use on-device Whisper, independently of the final provider.
Choose their model with **Live preview model**. The full transcript replaces
previews after Stop.

## OpenAI-compatible server

Choose **OpenAI-compatible server** and enter the model ID your server supports.
The daemon uploads the recorded mixed audio after Stop. A hosted service may
charge for this; a server you run on loopback can keep processing on your
machine. The provider must support:

- `POST {base_url}/audio/transcriptions` with multipart `file` and `model`.
- `response_format=verbose_json` and `timestamp_granularities[]=segment`.
- A JSON response with `text` and `segments`, each containing `start`, `end`
  (seconds relative to the uploaded file), and `text`.

Compatibility is a capability requirement, not a promise that every model at an
OpenAI-compatible endpoint supports this format. Text-only models cannot
preserve the meeting timeline and are rejected. Use a timestamp-capable model
from your service. The
[official API reference](https://developers.openai.com/api/reference/resources/audio/subresources/transcriptions/methods/create)
documents model-specific response formats; verify your provider's own contract
as well.

Configure these variables in the environment that launches `bruhs-capture`
(or the parent app/dev script that starts it), then restart the daemon:

| Variable | Value |
| --- | --- |
| `BRUHS_TRANSCRIPTION_BASE_URL` | API base URL, including its prefix, e.g. `https://your-service.example/v1`. |
| `BRUHS_TRANSCRIPTION_API_KEY` | Bearer credential for that service. Optional for a server that needs no authentication. |
| `BRUHS_TRANSCRIPTION_TIMEOUT_SECS` | Positive request deadline in seconds, chosen from your provider's SLA and maximum upload processing time. Required; no guessed default. |

The URL must use HTTPS, except HTTP on loopback (`127.0.0.1`, `::1`, or
`localhost`). Credentials, queries, and fragments in the URL are rejected.
Endpoint and credential changes require restarting the daemon; users can change
provider and model in settings without restarting. Only one compatible endpoint
is configured per daemon in this first implementation.

Keys stay in the native daemon and are never part of browser preferences or
capture-start bodies. The client refuses redirects, so a response cannot move
audio or credentials to a different service. Provider error bodies are not
included in logs or meeting events. Missing or invalid configuration fails
before recording devices open; HTTP failures are reported during finalization.
There is no automatic retry or fallback to a different provider.

## Long recordings and failures

Audio is streamed from the local WAV into PCM16 WAV uploads of at most 25 MB,
following the compatible API's documented
[file limit](https://developers.openai.com/api/docs/guides/speech-to-text#longer-inputs).
Memory for audio chunks stays bounded by the upload size; the whole meeting is
not loaded for remote transcription. Returned segment times are checked and
shifted into the full recording's timeline before the existing speaker pipeline
runs. The initial chunker splits on complete audio frames; sentence-aware chunk
boundaries and overlap handling are future quality work.

A provider failure retains the original audio and any saved transcript prefix,
and records the existing durable processing-failure receipt. Successful upload
and text generation alone do not count as successful transcription if the
response lacks valid timestamps. Recovery does not silently upload the recording
to another provider. Provider confidence remains the existing placeholder; it
is not a calibrated estimate.

The standing [transcription improvement goal](transcription-goal-2026-11-09.md)
tracks evaluation, better speech detection, additional provider capabilities,
and dedicated non-speech event detection. This change adds no yawn/burp detector
and makes no state-of-the-art accuracy claim.
