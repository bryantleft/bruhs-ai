import type { ExpoConfig } from 'expo/config';

const development = process.env.BRUHS_MOBILE_VARIANT === 'development';
// Reuse the team already selected in this workstation's Xcode account. The
// public team identifier is configurable; credentials remain in Xcode/Keychain.
const appleTeamId = (process.env.BRUHS_APPLE_TEAM_ID ?? 'KW4434WNW9').trim();
if (!/^[A-Z0-9]{10}$/.test(appleTeamId)) {
  throw new Error(
    'BRUHS_APPLE_TEAM_ID must be the 10-character Apple development team identifier.',
  );
}
const config: ExpoConfig = {
  name: development ? 'Bruhs Dev' : 'Bruhs',
  slug: 'bruhs-mobile',
  version: '0.1.0',
  scheme: 'bruhs',
  orientation: 'default',
  userInterfaceStyle: 'dark',
  icon: './assets/icon.png',
  ios: {
    appleTeamId,
    bundleIdentifier: development ? 'dev.bruhs.mobile.development' : 'dev.bruhs.mobile',
    supportsTablet: true,
    infoPlist: {
      UIFileSharingEnabled: false,
      NSLocalNetworkUsageDescription: 'Connect to the Bruhs computer you explicitly pair with.',
      ...(development ? { NSAppTransportSecurity: { NSAllowsLocalNetworking: true } } : {}),
    },
  },
  android: {
    package: development ? 'dev.bruhs.mobile.development' : 'dev.bruhs.mobile',
    allowBackup: false,
    adaptiveIcon: { foregroundImage: './assets/icon.png', backgroundColor: '#171310' },
  },
  web: { name: 'Bruhs mobile preview', favicon: './assets/icon.png', bundler: 'metro' },
  plugins: [
    './plugins/with-local-ios-signing.cjs',
    [
      'expo-camera',
      {
        cameraPermission: 'Scan the one-use pairing code shown by your Bruhs computer.',
        recordAudioAndroid: false,
      },
    ],
    ['expo-secure-store', { configureAndroidBackup: true }],
    [
      'expo-font',
      { fonts: ['./assets/MonaspaceNeon-Regular.otf', './assets/MonaspaceNeon-Medium.otf'] },
    ],
    [
      'expo-speech-recognition',
      {
        microphonePermission: 'Dictate a draft locally on this device.',
        speechRecognitionPermission: 'Transcribe a draft using on-device speech recognition.',
      },
    ],
    'expo-dev-client',
  ],
  updates: { enabled: false },
};
export default config;
