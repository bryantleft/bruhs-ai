# Local iOS signing

The mobile app reuses the Apple team already selected in this workstation's Xcode account: `KW4434WNW9`. `app.config.ts` sets Expo's supported [`ios.appleTeamId`](https://docs.expo.dev/versions/latest/config/app/#appleteamid) property. Set `BRUHS_APPLE_TEAM_ID` to another valid 10-character team identifier when building under a different authorized team. Credentials stay in Xcode and Keychain.

`plugins/with-local-ios-signing.cjs` reproduces Automatic signing in both generated app configurations. It applies the team only to signable application targets, clears stale manual-profile overrides and selects Apple Development for the device SDK. Simulator builds use ad-hoc signing without an Apple account. This survives native project regeneration instead of relying on an untracked manual Xcode setting.

## Commands

Run these in `bruhs-mobile/` with Node and the local Xcode/CocoaPods toolchain:

```sh
# Metadata only; does not contact the developer portal or build native code.
npm run ios:check

# Local production archive using the existing account saved in Xcode.
npm run ios:archive

# Development device build: explicitly select a reachable, already paired device.
BRUHS_IOS_DEVICE_ID='<Xcode device identifier>' npm run ios:device

# Optional compile-only archive. It cannot be installed or treated as signed.
npm run ios:archive:unsigned
```

`ios:archive` defaults to the production bundle `dev.bruhs.mobile`; `ios:device` defaults to `dev.bruhs.mobile.development`. Override `BRUHS_MOBILE_VARIANT=development|production` explicitly when needed. The driver regenerates only iOS, installs its Pods and builds with bounded parallelism in `build/ios-device`. It never selects or pairs a physical device automatically, installs an app, launches a camera/microphone, exports private keys or requests a store upload.

The signed modes pass `-allowProvisioningUpdates`. Xcode's installed help documents that this permits its existing developer account to create/update app IDs, certificates and profiles for automatically signed targets. The device mode additionally permits registration of the explicitly selected destination. No EAS, cloud build or App Store publishing command is used. Logs live in the ignored `build/ios-signing/` directory with mode `0600`; account emails and common credential forms are redacted before console or file output.

An existing App Store Connect team API key can authenticate provisioning independently of the saved Xcode session. Set all three metadata variables for the local command:

```sh
BRUHS_APPLE_API_KEY_PATH='/path/to/existing/AuthKey_ABC1234567.p8' \
BRUHS_APPLE_API_KEY_ID='ABC1234567' \
BRUHS_APPLE_API_ISSUER_ID='<issuer UUID>' npm run ios:archive
```

The helper passes the existing key's file path and identifiers directly to Xcode's [supported API-key authentication flags](https://developer.apple.com/documentation/xcode-release-notes/xcode-13-release-notes). It does not copy or print the private key, place it in Expo config or include it in the app. An incomplete/malformed metadata set fails before a build starts.

This workstation's existing key metadata is saved outside the project at `~/Library/Application Support/bruhs-mobile/apple-signing.json` with mode `0600`. It contains only `keyPath`, `keyId` and `issuerId`; the private key stays in its original protected file. The driver loads this metadata when no API variables are set, so `npm run ios:archive` can reuse the verified setup directly. All three environment variables override it together. If neither source is configured, signed commands use the saved Xcode account. Invalid local metadata fails actionably instead of silently switching accounts.

Each local build command owns its process group. Interrupting the driver forwards the signal to that group and waits for the child before returning; a signing-log write failure stops the active group as well. Fake-tool subprocess checks verified both cancellation and a simulated disk-write failure without invoking Xcode, CocoaPods or Apple.

The installed Expo 58 CLI checks whether a development team is already configured before preparing signing. In that path it does not add its provisioning-update flags. The explicit local driver therefore supplies the missing flag while preserving CocoaPods' unsigned library/resource target settings. Signing identity/team settings are scoped to the generated app target, rather than passed globally to every Pod.

## Observed state on 2026-11-13

- `npm run ios:check` found one local Developer ID Application identity and no local Apple Development or Apple Distribution identity. This inventory alone does not establish the account's iOS access; [Apple certificate types serve different purposes](https://developer.apple.com/help/account/certificates/certificates-overview).
- An actual production archive attempt regenerated the `Bruhs` project, installed the native dependencies and requested automatic provisioning through the saved Xcode account. It exited with Xcode status `65`. The app target reported: **“Unable to log in with account '[Xcode account]'. The login details for account '[Xcode account]' were rejected.”** It also reported no iOS App Development profile for `dev.bruhs.mobile`.
- A follow-up located an already installed App Store Connect team API key and existing issuer metadata. Apple's read-only certificate API accepted it, and both returned certificates belonged to the configured team. The local archive retry with that key created a local Apple Development identity and **succeeded**, avoiding the stale saved-login dependency.
- This is a verified Xcode account-session blocker. A working macOS notarization profile, cached team flags, or the absence of a local iOS certificate do not independently prove current iOS membership/role status. No conclusion about membership is made until Xcode can authenticate and return a current provisioning result.
- The private device inventory contained one known paired iPhone with `tunnelState=unavailable`. It was not reachable for installation or physical-device acceptance. No forced pairing, privacy permission or audio/video capture was attempted.
- The redacted attempt log is `bruhs-mobile/build/ios-signing/archive-production-2026-11-14T06-09-36.744Z.log`; its permissions were verified as `0600`, and the captured log contains no unredacted email address. The separate unsigned attempt was stopped at Build Preparation to avoid competing with Android compilation and account refresh; it is not recorded as a successful archive.
- The Release archive is `bruhs-mobile/build/ios-signing/Bruhs.xcarchive`. `codesign --verify --deep --strict` passes. The embedded profile and entitlements match `KW4434WNW9.dev.bruhs.mobile`, include one registered device and expire on 2027-11-14. The app embeds its JavaScript bundle. This archive uses **Apple Development** provisioning (`get-task-allow=true`); it is not an App Store distribution export. Verification found none of the API-key metadata in app config or JavaScript. The private key was not copied/exported, and no cloud build or upload was used.
- iOS signing-log redaction tests bring the mobile test suite to **33 passing tests**. Strict TypeScript passes. The previous real iOS simulator build and native companion acceptance remain valid. Local archive and read-only API receipts are retained in the ignored `build/ios-signing/verification.json` and `api-readiness.json` files.

The final Android-tested layout was rebuilt into the signed Release archive at 2026-11-14 10:15:23 UTC. Its source hashes stayed unchanged from build start through verification, its JavaScript bundle is embedded, and deep/strict signature verification passes. The current receipt is `build/ios-signing/verification-current.json`; the earlier archive is preserved separately. This verifies the current iOS build and signing, while the latest 100%/200% keyboard acceptance ran on Android.

The signed app has the exact application identifier `KW4434WNW9.dev.bruhs.mobile`; Xcode selected the same team's wildcard development profile. A wildcard profile can cover an exact signed app identifier when its prefix and pattern match, as described in [Apple's entitlement troubleshooting note](https://developer.apple.com/library/archive/technotes/tn2415/). Verification checks both the exact signed identifier and the profile match.

Repeat the signed archive command with the existing API-key metadata to use the verified path. The saved Xcode account can be refreshed independently for interactive account use. Physical installation and hardware acceptance still require the paired iPhone to become reachable. App Store distribution/export was not requested by these local validation commands.
