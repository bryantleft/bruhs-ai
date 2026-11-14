import { useRef, useState } from 'react';
import { CameraView, useCameraPermissions } from 'expo-camera';
import { Text, View } from 'react-native';
import { type Pairing, parsePairing } from './domain';
import { Button, Field, Notice, PageScroll } from './components';
import { styles as s } from './theme';

export function PairScreen({
  busy,
  initialAuthority,
  incoming,
  onIncomingConsumed,
  onDesktop,
  onTasks,
}: Readonly<{
  busy: boolean;
  initialAuthority: 'desktop' | 'tasks';
  incoming: Pairing | null;
  onIncomingConsumed: () => void;
  onDesktop: (pair: Pairing, name: string) => Promise<boolean>;
  onTasks: (origin: string, name: string, token: string) => Promise<boolean>;
}>) {
  const [kind, setKind] = useState<'desktop' | 'tasks'>(initialAuthority);
  const [link, setLink] = useState(''),
    [name, setName] = useState('My computer'),
    [origin, setOrigin] = useState(''),
    [token, setToken] = useState(''),
    [problem, setProblem] = useState('');
  const [pairing, setPairing] = useState<Pairing | null>(null),
    [scanning, setScanning] = useState(false);
  const [permission, askCamera] = useCameraPermissions();
  const scanned = useRef(false);
  const offer = incoming ?? pairing;
  function read(value: string): void {
    const parsed = parsePairing(value, __DEV__);
    if (parsed.ok) {
      setPairing(parsed.value);
      setLink('');
      setScanning(false);
      setProblem('');
    } else setProblem(parsed.error.message);
  }
  async function scan(): Promise<void> {
    if (!permission?.granted) {
      const result = await askCamera();
      if (!result.granted) {
        setProblem('Camera access was not granted. Paste the pairing link instead.');
        return;
      }
    }
    scanned.current = false;
    setScanning(true);
  }
  return (
    <PageScroll keyboardShouldPersistTaps="handled" contentContainerStyle={[s.page, s.narrow]}>
      <View style={s.empty}>
        <Text style={s.small}>BRUHS / MOBILE</Text>
        <Text style={s.hero}>Your desk,{'\n'}within reach.</Text>
        <Text style={s.text}>
          Keep working with the agents on your computer. Your computer stays in charge and needs to
          remain awake.
        </Text>
      </View>
      {problem ? <Notice error>{problem}</Notice> : null}
      {offer ? (
        <View style={s.permission}>
          <Text style={s.subheading}>Confirm your computer</Text>
          <Text selectable style={s.text}>
            {offer.origin}
          </Text>
          <Text style={s.small}>
            Only continue for a pairing code you created in Bruhs on a trusted computer. This
            one-use link grants the permissions chosen there.
          </Text>
          <Field label="Computer name" value={name} onChange={setName} maxLength={128} />
          <Button
            primary
            disabled={busy || !name.trim()}
            onPress={() =>
              void onDesktop(offer, name).then((ok) => {
                if (ok) {
                  setPairing(null);
                  onIncomingConsumed();
                }
              })
            }
          >
            Pair this computer
          </Button>
          <Button
            quiet
            disabled={busy}
            onPress={() => {
              setPairing(null);
              onIncomingConsumed();
            }}
          >
            Cancel pairing
          </Button>
        </View>
      ) : (
        <>
          <View style={s.row}>
            <Button onPress={() => setKind('desktop')} primary={kind === 'desktop'}>
              Desktop session
            </Button>
            <Button onPress={() => setKind('tasks')} primary={kind === 'tasks'}>
              Task host
            </Button>
          </View>
          {kind === 'desktop' ? (
            <View style={{ gap: 16 }}>
              <Text style={s.text}>
                On your computer, open Bruhs → Mobile access and create a pairing code.
              </Text>
              {scanning ? (
                <>
                  <View style={s.camera}>
                    <CameraView
                      style={{ flex: 1 }}
                      barcodeScannerSettings={{ barcodeTypes: ['qr'] }}
                      onBarcodeScanned={(event) => {
                        if (!scanned.current) {
                          scanned.current = true;
                          read(event.data);
                        }
                      }}
                    />
                  </View>
                  <Button onPress={() => setScanning(false)}>Close camera</Button>
                </>
              ) : (
                <Button onPress={() => void scan()} disabled={busy}>
                  Scan pairing code
                </Button>
              )}
              <Field
                label="Or paste the one-use pairing link"
                value={link}
                onChange={setLink}
                placeholder="https://your-computer/#pair=…"
                secret
                keyboardType="url"
                testID="pair-link"
              />
              <Button primary onPress={() => read(link)} disabled={busy || !link.trim()}>
                Review pairing
              </Button>
            </View>
          ) : (
            <View style={{ gap: 16 }}>
              <Text style={s.text}>
                Use a task access token issued in Workbench → Tasks → Phone and guest access. This
                connects to durable tasks, separately from desktop chats.
              </Text>
              <Field label="Computer name" value={name} onChange={setName} maxLength={128} />
              <Field
                label="Trusted HTTPS task address"
                value={origin}
                onChange={setOrigin}
                keyboardType="url"
                placeholder="https://your-task-host"
              />
              <Field label="Owner or guest access token" value={token} onChange={setToken} secret />
              <Button
                primary
                disabled={busy || !name.trim() || !origin.trim() || !token.trim()}
                onPress={() =>
                  void onTasks(origin, name, token).then((ok) => {
                    if (ok) setToken('');
                  })
                }
              >
                Connect task host
              </Button>
            </View>
          )}
        </>
      )}
      <View style={s.section}>
        <Text style={s.small}>
          Direct connections over your trusted HTTPS address, private network or VPN. No managed
          relay or cloud worker. A loopback address points to this phone, not your computer.
        </Text>
      </View>
    </PageScroll>
  );
}
