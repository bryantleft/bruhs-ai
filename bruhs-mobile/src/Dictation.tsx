import { useEffect, useRef, useState } from 'react';
import { AppState, Platform, Text, View } from 'react-native';
import {
  ExpoSpeechRecognitionModule as speech,
  useSpeechRecognitionEvent,
} from 'expo-speech-recognition';
import { Button } from './components';
import { RecordingGate } from './recordingGate';
import { styles as s } from './theme';

export function Dictation({
  value,
  onText,
}: Readonly<{ value: string; onText: (text: string) => void }>) {
  const [listening, setListening] = useState(false),
    [preparing, setPreparing] = useState(false),
    [message, setMessage] = useState('');
  const gate = useRef(new RecordingGate()).current;
  const active = useRef(false),
    transcript = useRef(''),
    current = useRef(value),
    apply = useRef(onText);
  current.current = value;
  apply.current = onText;
  const available = Platform.OS !== 'web' && speech.supportsOnDeviceRecognition();
  useSpeechRecognitionEvent('result', (event) => {
    if (active.current && gate.mounted && AppState.currentState === 'active')
      transcript.current = event.results[0]?.transcript ?? '';
  });
  useSpeechRecognitionEvent('end', () => {
    if (!active.current || !gate.mounted || AppState.currentState !== 'active') return;
    active.current = false;
    setListening(false);
    const text = transcript.current.trim();
    if (text) {
      const draft = current.current;
      apply.current(`${draft}${draft && !/\s$/.test(draft) ? ' ' : ''}${text}`);
      setMessage('Added to your draft. Review before sending.');
    } else setMessage('No speech was recognized. Your draft is unchanged.');
  });
  useSpeechRecognitionEvent('error', (event) => {
    if (active.current && gate.mounted) {
      active.current = false;
      setListening(false);
      setMessage(`Local dictation stopped: ${event.message}`);
    }
  });
  const cancel = (): void => {
    const hadActivity = active.current || gate.starting;
    gate.cancel();
    const recording = active.current;
    active.current = false;
    transcript.current = '';
    if (recording) speech.abort();
    if (gate.mounted) {
      setPreparing(false);
      setListening(false);
      if (hadActivity) setMessage('Dictation cancelled. Tap Dictate again when you are ready.');
    }
  };
  useEffect(() => {
    gate.mount(AppState.currentState === 'active');
    const listener = AppState.addEventListener('change', (state) => {
      if (state !== 'active') cancel();
      gate.setForeground(state === 'active');
    });
    return () => {
      gate.unmount();
      cancel();
      listener.remove();
    };
  }, [gate]);
  async function start(): Promise<void> {
    if (!available) {
      setMessage('On-device speech recognition is unavailable. No audio was sent to a server.');
      return;
    }
    if (active.current) return;
    const attempt = gate.begin();
    if (attempt === null) return;
    setPreparing(true);
    const mayStart = (): boolean => gate.current(attempt) && AppState.currentState === 'active';
    try {
      // The maintained module documents microphone-only consent for required
      // on-device recognition. Speech permission is for Apple's network path.
      const permission = await speech.requestMicrophonePermissionsAsync();
      if (!mayStart()) return;
      if (!permission.granted) {
        setMessage('Microphone access was not granted. Enable it in device settings.');
        return;
      }
      if (Platform.OS === 'android') {
        const locales = await speech.getSupportedLocales({});
        if (!mayStart()) return;
        if (!locales.installedLocales.some((locale) => /^en[-_]US$/i.test(locale))) {
          setMessage(
            'Install the English (US) offline speech model in your device settings. No cloud recognition was used.',
          );
          return;
        }
      }
      if (!mayStart()) return;
      transcript.current = '';
      active.current = true;
      setListening(true);
      setMessage('Listening on this device.');
      speech.start({
        lang: 'en-US',
        requiresOnDeviceRecognition: true,
        interimResults: true,
        continuous: false,
        addsPunctuation: true,
        recordingOptions: { persist: false },
      });
    } catch {
      if (mayStart()) {
        active.current = false;
        setListening(false);
        setMessage(
          'Local dictation could not start. Check the device’s offline speech and microphone settings.',
        );
      }
    } finally {
      if (gate.finish(attempt)) setPreparing(false);
    }
  }
  return (
    <View style={{ gap: 8, flex: 1, minWidth: 0 }}>
      <View style={s.row}>
        {listening ? (
          <>
            <Button onPress={() => speech.stop()}>Stop dictation</Button>
            <Button onPress={cancel} danger>
              Cancel recording
            </Button>
          </>
        ) : (
          <Button onPress={() => void start()} disabled={!available || preparing} quiet>
            {preparing ? 'Preparing dictation…' : 'Dictate on device'}
          </Button>
        )}
      </View>
      {message ? (
        <Text style={s.small} accessibilityLiveRegion="polite">
          {message}
        </Text>
      ) : Platform.OS === 'web' ? (
        <Text style={s.small}>Native app only.</Text>
      ) : null}
    </View>
  );
}
