# Bruhs mobile

Native iOS and Android companion built with Expo 58, React Native and TypeScript. Agents and credentials for providers stay on your computer.

```sh
npm ci
BRUHS_MOBILE_VARIANT=development npm run ios
# With a local Android SDK and emulator/device:
BRUHS_MOBILE_VARIANT=development npm run android
```

Use a local development build; the speech module requires native compilation. Builds use your installed Xcode/Android toolchain. There is no EAS or managed relay configuration.

[Local Android setup and synthetic emulator checks](ANDROID.md) include Android Studio toolchain discovery without changing your shell profile.

[Local iOS team, signing and archive setup](../docs/native-mobile-ios-signing.md) reuse the existing Xcode account and distinguish simulator, device and unsigned artifacts.

See [setup, protocol, validation and remaining device checks](../docs/native-mobile-app.md). Desktop session pairing and durable task tokens are separate connections.

```sh
npm run typecheck
npm test
npm run export:web
```

The web target is a diagnostic preview with memory-only credentials. The mobile app uses native views and device secure storage.
