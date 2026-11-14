# Local Android builds

`npm run android` uses the installed Expo CLI and Android toolchain. It finds the
SDK from `ANDROID_HOME`, `ANDROID_SDK_ROOT`, or Android Studio's standard SDK
directory. On macOS it also finds Android Studio's bundled Java runtime when
`JAVA_HOME` is unset. These paths apply only to the child command; the script
does not change your shell profile or system settings.

```sh
npm ci
node scripts/android-local.mjs --doctor
BRUHS_MOBILE_VARIANT=development npm run android -- --device
```

Choose your intended emulator or device from Expo's selector. If Java or the SDK
lives elsewhere, set `JAVA_HOME` and `ANDROID_HOME` for this command. If both
Android SDK variables are set, they must resolve to the same directory.

The locked React Native 0.88.1 source specifies compile SDK 37, Build-Tools
37.0.0, NDK 27.1.12297006 and Android Gradle Plugin 9.2.1. The generated project
uses Gradle 9.4.1. Install those SDK components through Android Studio if the
local build reports them missing. The local check on 2026-11-13 used Android
Studio's Java 21.0.6 and the already installed SDK; these tools were off `PATH`,
not absent.

For another run using an existing development APK:

```sh
BRUHS_MOBILE_VARIANT=development npm run android -- \
  --binary android/app/build/outputs/apk/debug/app-debug.apk --device
```

The debug APK uses a development signing key and loads JavaScript from Metro.
It is not a store distribution artifact. The Expo template's release signing
configuration also needs a developer-owned release keystore before distributing
Android releases; selecting a release build alone does not configure that key.

## Synthetic native acceptance

The test host only serves synthetic state; it never opens a provider, repository
or shell. Use a fresh emulator and its explicit adb serial. Preserve existing
AVDs and their app data.

```sh
BRUHS_FIXTURE_PORT=18443 node test/host-fixture.mjs
# In another terminal:
BRUHS_MOBILE_VARIANT=development npm start -- --localhost --port 18442
# Forward literal loopback only on the intended emulator:
adb -s <acceptance-emulator-serial> reverse tcp:18442 tcp:18442
adb -s <acceptance-emulator-serial> reverse tcp:18443 tcp:18443
```

Open the Metro project in the development client, paste this synthetic offer,
review the displayed loopback origin and pair:

```text
http://127.0.0.1:18443/#pair=synthetic_bruhs_pairing_offer_for_testing_1234
```

Check conversation and diff review, draft restoration after an app restart,
prompt submission, an interrupted approval's exact retry, read-only scopes,
offline controls and revocation. Fixture state can be inspected or changed
through its local `/__fixture/state` endpoint; the supported controls and fake
tokens are defined in `test/host-fixture.mjs`.

Development builds permit literal loopback HTTP for this fixture. Physical
devices require a reachable trusted HTTPS host. Emulator checks do not verify a
physical camera, microphone, offline recognition model, or device TLS setup.

## Verified on 2026-11-13

The local ARM64 development APK built successfully with 353 Gradle tasks in
12 minutes 1 second. The 68,545,227-byte APK passed `apksigner verify` and
installed as `dev.bruhs.mobile.development` on an isolated API 36 emulator.
The installed SDK and Android Studio Java were sufficient; no SDK installation
or cloud build was needed.

Native checks used that APK with the current-source Metro bundle and the
synthetic host. They covered manual pairing and origin review, native password
masking, saved connections and drafts across process restarts, diff review,
keyboard avoidance, prompt acknowledgement, read-only scopes, offline state,
reconnection and revocation. A reserved approval survived a process restart;
two identical requests retained the same UUID, sequence and action, producing
one dispatch. Revocation removed the active transcript and mutation controls
and displayed the pairing recovery screen.

The disposable emulator and its 2.54 GiB of data were removed after testing;
existing AVDs were preserved. Ignored local evidence remains in
`build/android-apk.log` and `build/android-acceptance/`, including the APK
metadata, assertions, fixture receipts and screenshots. The APK remains at
`android/app/build/outputs/apk/debug/app-debug.apk`.

The final accessibility follow-up on 2026-11-14 passes at 100% and 200% text.
It verifies first focus and repeated keyboard reopening, fresh approvals,
pending-command locks, diff/draft preservation, exact retry and acknowledgement,
scrollable offline content, and revoked recovery. Send sits beside the field and
above its lower padding, remaining fully visible with the keyboard. Reading
elsewhere preserves its anchor when a synthetic message arrives. Final receipts
are under `build/android-keyboard-centered-34323f4f/`; earlier failed layouts
remain separately preserved. The same development APK loaded the exact tested
Metro source, without a new native build or hardware permission grant.

These runs did not exercise physical hardware permissions, trusted
physical-device TLS, offline speech models or native durable-task owner/guest
flows. The APK is a development build with Metro, not an Android
distribution-signing result.
