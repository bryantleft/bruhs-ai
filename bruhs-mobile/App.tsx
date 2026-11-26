import { type ReactNode, useCallback, useContext, useEffect, useRef, useState } from 'react';
import {
  ActivityIndicator,
  Alert,
  Keyboard,
  KeyboardAvoidingView,
  Linking,
  Platform,
  Pressable,
  ScrollView,
  type ScrollViewInstance,
  type TextInputInstance,
  Text,
  View,
  useWindowDimensions,
} from 'react-native';
import { SafeAreaProvider, useSafeAreaInsets } from 'react-native-safe-area-context';
import { StatusBar } from 'expo-status-bar';
import { useFonts } from 'expo-font';
import { useCompanion } from './src/useCompanion';
import { useLinkedDraftTarget } from './src/useLinkedDraftTarget';
import { type Pairing, type Task, parsePairing, running } from './src/domain';
import { type LinkedPrompt, parseIncomingPromptLink, parsePromptFragment, pullRequestUrl } from './src/promptLink';
import { ephemeralStorage } from './src/storage';
import { Button, Field, Notice, PageNotices, PageScroll, StateBadge } from './src/components';
import { QuestionCard } from './src/QuestionCard';
import { Dictation } from './src/Dictation';
import { PairScreen } from './src/PairScreen';
import { color, styles as s } from './src/theme';

type Companion = ReturnType<typeof useCompanion>;
type Tab = 'work' | 'hosts' | 'access';
type PromptHandoff = {
  link: LinkedPrompt;
  destination: { hostId: string; threadId: string } | null;
  placed: boolean;
  modelFallback?: string;
};

export default function App() {
  const [loaded, error] = useFonts({
    Neon: require('./assets/MonaspaceNeon-Regular.otf'),
    NeonMedium: require('./assets/MonaspaceNeon-Medium.otf'),
  });
  return (
    <SafeAreaProvider>
      <StatusBar style="light" />
      {error ? (
        <View style={[s.screen, s.page]}>
          <Text style={{ color: color.text }}>
            Bruhs could not load its bundled fonts. Reinstall this build.
          </Text>
        </View>
      ) : loaded ? (
        <CompanionApp />
      ) : (
        <View style={[s.screen, { justifyContent: 'center' }]}>
          <ActivityIndicator color={color.accent} accessibilityLabel="Opening Bruhs" />
        </View>
      )}
    </SafeAreaProvider>
  );
}

function CompanionApp() {
  const model = useCompanion(),
    safe = useSafeAreaInsets();
  const [tab, setTab] = useState<Tab>('hosts'),
    [adding, setAdding] = useState(false),
    [incoming, setIncoming] = useState<Pairing | null>(null),
    [linkError, setLinkError] = useState('');
  const [keyboardOpen, setKeyboardOpen] = useState(false);
  const [handoff, setHandoff] = useState<PromptHandoff | null>(null);
  const lastHandoffRoute = useRef<{ link: LinkedPrompt; hostId: string | null } | null>(null);
  useEffect(() => {
    const shown = Keyboard.addListener('keyboardDidShow', () => setKeyboardOpen(true));
    const hidden = Keyboard.addListener('keyboardDidHide', () => setKeyboardOpen(false));
    return () => {
      shown.remove();
      hidden.remove();
    };
  }, []);
  useEffect(() => {
    let live = true;
    let receivedEvent = false;
    const open = (value: string | null | undefined) => {
      if (!live || !value) return;
      let parsedPrompt = parseIncomingPromptLink(value);
      if (parsedPrompt.kind === 'ignored' && Platform.OS === 'web' && /^https?:/.test(value)) {
        try {
          parsedPrompt = parsePromptFragment(new URL(value).hash);
        } catch { /* The pairing parser handles malformed pairing links. */ }
      }
      if (parsedPrompt.kind === 'prompt') {
        setLinkError('');
        setHandoff({ link: parsedPrompt.link, destination: null, placed: false });
        if (Platform.OS === 'web' && window.location.href === value) {
          window.history.replaceState(window.history.state, '', window.location.pathname + window.location.search);
        }
        return;
      }
      if (parsedPrompt.kind === 'invalid') {
        setHandoff(null);
        setLinkError(parsedPrompt.message);
        return;
      }
      if (!/^(bruhs:|https?:)/.test(value) || !value.includes('pair')) return;
      const parsed = parsePairing(value, __DEV__);
      if (parsed.ok) {
        setIncoming(parsed.value);
        setAdding(true);
        setTab('hosts');
      } else setLinkError(parsed.error.message);
    };
    if (Platform.OS === 'web') {
      const browserRoute = () => open(window.location.href);
      browserRoute();
      window.addEventListener('hashchange', browserRoute);
      return () => window.removeEventListener('hashchange', browserRoute);
    }
    void Linking.getInitialURL().then((url) => { if (!receivedEvent) open(url); });
    const listener = Linking.addEventListener('url', (event) => {
      receivedEvent = true;
      open(event.url);
    });
    return () => {
      live = false;
      listener.remove();
    };
  }, []);
  const { host, chooseItem, placeLinkedDraft } = model;
  const linked = handoff?.link;
  const requestedTarget = linked?.pullRequest ? pullRequestUrl(linked.pullRequest)
    : [linked?.repository, linked?.provider, linked?.model].filter(Boolean).join(' · ');
  const specializedPrompt = Boolean(requestedTarget);
  const targetReview = useLinkedDraftTarget(model, linked?.threadId === null ? linked : undefined);
  const openingTarget = linked?.threadId === null && !handoff?.destination
    && (specializedPrompt || targetReview.supported);
  const routeLink = handoff?.placed || openingTarget || (handoff?.destination && !linked?.threadId) ? null : linked;
  const prepared = targetReview.status;
  useEffect(() => {
    if (!linked || handoff?.destination || !host || prepared?.status !== 'ready') return;
    const text = prepared.without_project && linked.pullRequest
      ? [pullRequestUrl(linked.pullRequest), linked.text].filter(Boolean).join('\n\n')
      : linked.text;
    setHandoff((current) => current?.link === linked ? {
      ...current,
      link: { ...linked, text },
      destination: { hostId: host.id, threadId: prepared.thread_id },
      modelFallback: prepared.model_fallback ? prepared.model : undefined,
    } : current);
    chooseItem({ id: prepared.thread_id });
    setTab('work');
  }, [linked, handoff?.destination, host, prepared, chooseItem]);
  useEffect(() => {
    if (!routeLink) return;
    const hostId = host?.id ?? null;
    if (lastHandoffRoute.current?.link === routeLink && lastHandoffRoute.current.hostId === hostId) return;
    lastHandoffRoute.current = { link: routeLink, hostId };
    const canOpen = host && (routeLink.threadId === null || host.authority === 'desktop');
    setTab(canOpen ? 'work' : 'hosts');
    const threadId = routeLink.threadId;
    if (!canOpen || !threadId) {
      if (canOpen) chooseItem(null);
      return;
    }
    chooseItem({ id: threadId });
    setHandoff((request) => request?.link === routeLink
      ? { ...request, destination: { hostId: host.id, threadId }, placed: false }
      : request);
  }, [routeLink, host, chooseItem]);

  const onChooseConversation = (threadId: string) => {
    if (host && handoff?.link.threadId === null && !handoff.placed && !openingTarget
      && (!handoff.destination || (!specializedPrompt && !targetReview.supported))) {
      setHandoff({ ...handoff, destination: { hostId: host.id, threadId } });
    }
    chooseItem({ id: threadId });
  };
  const destination = handoff?.destination;
  const bound = destination && host?.id === destination.hostId && model.selection?.id === destination.threadId;
  const destinationOpen = bound && host
    && (host.authority === 'desktop'
      ? model.thread?.thread.id === destination.threadId
      : model.tasks?.tasks.some((task) => task.id === destination.threadId));
  const draftReady = Boolean(destinationOpen && model.phase === 'online');
  const needsReplacement = Boolean(linked?.text && model.draft.trim() && model.draft !== linked.text);
  const place = useCallback((request: PromptHandoff, expected: string): void => {
    const target = request.destination;
    if (target && (!request.link.text || placeLinkedDraft(target.hostId, target.threadId, expected, request.link.text))) {
      setHandoff((current) => current === request ? { ...request, placed: true } : current);
    }
  }, [placeLinkedDraft]);
  useEffect(() => {
    if (handoff && !handoff.placed && draftReady && !needsReplacement) place(handoff, model.draft);
  }, [handoff, draftReady, needsReplacement, model.draft, place]);
  const online = model.phase === 'online';
  const connection =
    model.phase === 'none'
      ? 'Local first'
      : online
        ? 'Connected'
        : model.phase === 'connecting'
          ? 'Connecting'
          : model.phase === 'expired'
            ? 'Pair again'
            : 'Offline';
  const promptReview = handoff && (!handoff.placed || destinationOpen) ? (
    <View style={s.banner} testID="prompt-handoff">
      <Notice>{openingTarget ? (
        !targetReview.supported ? 'Choose a paired computer with linked draft access.'
        : prepared?.status === 'preparing' ? 'Preparing the linked draft…'
        : prepared?.status === 'missing_repository' ? `${prepared.repository} is missing on this computer.`
        : prepared?.status === 'checkout' ? `Check out this pull request for ${prepared.project}?`
        : 'Open a draft on the paired computer.'
      ) : handoff.placed ? (handoff.link.text ? 'Prompt added from a link. Review the draft before sending.' : 'Linked draft ready.')
        : !host || (linked?.threadId && host.authority !== 'desktop') ? 'Choose the computer for this linked prompt.'
        : draftReady ? 'Replace the existing draft with this prompt?'
        : bound && model.problem ? 'Could not open the linked conversation on this computer.'
        : bound ? 'Opening the conversation for this prompt…'
        : 'Choose a conversation for this linked prompt.'}</Notice>
      {requestedTarget ? (
        <Text selectable style={s.small}>{requestedTarget}</Text>
      ) : null}
      {!handoff.placed && handoff.link.text ? (
        <ScrollView nestedScrollEnabled style={{ maxHeight: 112, flexGrow: 0 }}>
          <Text selectable style={s.text}>{handoff.link.text}</Text>
        </ScrollView>
      ) : null}
      {handoff.link.removedHiddenCharacters ? <Text style={s.small}>Hidden characters were removed.</Text> : null}
      {handoff.modelFallback ? <Text style={s.small}>Requested model unavailable. Using {handoff.modelFallback}.</Text> : null}
      {openingTarget && prepared?.status === 'checkout' ? <Text style={s.small}>Saved setup commands run in the new worktree.</Text> : null}
      {openingTarget && prepared?.status === 'missing_repository' ? (
        <Text style={s.small}>{prepared.clone_parent
          ? `Clone into ${prepared.clone_parent} on ${host?.name}.`
          : 'Choose a project start folder in the computer’s settings to clone here.'}</Text>
      ) : null}
      {openingTarget && prepared && 'error' in prepared && prepared.error ? <Notice error>{prepared.error}</Notice> : null}
      {openingTarget && prepared?.status === 'failed' ? <Notice error>{prepared.message}</Notice> : null}
      <View style={s.row}>
        <Button quiet disabled={openingTarget && prepared?.status === 'preparing'} onPress={() => setHandoff(null)}>
          {handoff.placed ? 'Dismiss' : draftReady && needsReplacement ? 'Keep draft' : 'Cancel'}
        </Button>
        {openingTarget && targetReview.supported ? (
          prepared?.status === 'checkout' || prepared?.status === 'missing_repository' ? <>
            <Button quiet disabled={!targetReview.canPrepare} onPress={() => targetReview.confirm('without_project')}>Use without project</Button>
            {prepared.status === 'checkout' ? <Button disabled={!targetReview.canPrepare} onPress={() => targetReview.confirm('checkout')}>Check out</Button>
              : prepared.clone_parent ? <Button disabled={!targetReview.canPrepare} onPress={() => targetReview.confirm('clone')}>Clone</Button> : null}
          </> : (!prepared || prepared.status === 'failed') ? (
            <Button disabled={!targetReview.canPrepare} onPress={() => {
              chooseItem(null);
              setTab('work');
              targetReview.prepare();
            }}>{`Open on ${host?.name ?? 'computer'}`}</Button>
          ) : null
        ) : openingTarget ? (
          <Button onPress={() => {
            const { text, removedHiddenCharacters, pullRequest } = handoff.link;
            const draft = pullRequest ? [pullRequestUrl(pullRequest), text].filter(Boolean).join('\n\n') : text;
            setHandoff({ link: { threadId: null, text: draft, removedHiddenCharacters }, destination: null, placed: false });
          }}>Use as a draft</Button>
        ) : !handoff.placed && draftReady && needsReplacement ? (
          <Button onPress={() => place(handoff, model.draft)}>Replace draft</Button>
        ) : null}
      </View>
    </View>
  ) : null;
  const reviewInComposer = tab === 'work' && destinationOpen;
  const notices = (
    <>
      {reviewInComposer ? null : promptReview}
      {model.problem || linkError ? (
        <View style={s.banner}>
          <Notice error>{model.problem || linkError}</Notice>
          <Button
            onPress={() => {
              model.clearProblem();
              setLinkError('');
            }}
            quiet
          >
            Dismiss message
          </Button>
        </View>
      ) : null}
      {model.host && !online && tab === 'work' ? (
        <View style={s.banner}>
          <Text style={s.small}>
            {model.phase === 'expired'
              ? 'Access ended. Create a new pairing on the computer.'
              : 'Showing the last received state. Sending and approvals are paused until the host reconnects.'}
          </Text>
          <Button
            disabled={model.busy || model.phase === 'expired'}
            onPress={() => void model.refresh()}
            quiet
          >
            Reconnect
          </Button>
        </View>
      ) : null}
    </>
  );
  return (
    <KeyboardAvoidingView
      style={[s.screen, { paddingTop: safe.top }]}
      behavior={Platform.OS === 'ios' ? 'padding' : 'height'}
    >
      <View style={s.frame}>
        <View style={s.header}>
          <Text style={s.brand}>
            bruhs<Text style={s.brandAccent}>.</Text>
          </Text>
          <View style={s.status}>
            {model.host ? (
              <Text numberOfLines={1} style={[s.small, { maxWidth: '100%' }]}>
                {model.host.name}
              </Text>
            ) : null}
            <View style={[s.row, { gap: 7, justifyContent: 'flex-end' }]}>
              <View style={[s.dot, online && s.online]} />
              <Text style={s.small}>{connection}</Text>
            </View>
          </View>
        </View>
        <PageNotices.Provider value={notices}>
          <View style={s.grow}>
            {tab === 'work' ? (
              model.host ? (
                <WorkScreen
                  model={model}
                  onChooseConversation={onChooseConversation}
                  promptReview={reviewInComposer ? promptReview : null}
                  onPair={() => {
                    setAdding(true);
                    setTab('hosts');
                  }}
                />
              ) : (
                <Empty
                  onAdd={() => {
                    setTab('hosts');
                    setAdding(true);
                  }}
                />
              )
            ) : null}
            {tab === 'hosts' ? (
              adding || !model.hosts.length ? (
                <PairScreen
                  busy={model.busy}
                  initialAuthority={model.host?.authority ?? 'desktop'}
                  incoming={incoming}
                  onIncomingConsumed={() => setIncoming(null)}
                  onDesktop={async (pair, name) => {
                    const ok = await model.addDesktop(pair, name);
                    if (ok) {
                      setAdding(false);
                      setTab('work');
                    }
                    return ok;
                  }}
                  onTasks={async (origin, name, token) => {
                    const ok = await model.addTasks(origin, name, token);
                    if (ok) {
                      setAdding(false);
                      setTab('work');
                    }
                    return ok;
                  }}
                />
              ) : (
                <PageScroll contentContainerStyle={[s.page, s.narrow]}>
                  <Text style={s.heading}>Your computers</Text>
                  <Text style={s.text}>
                    Choose where your work runs. Each connection keeps its own credentials and
                    drafts.
                  </Text>
                  {model.hosts.map((host) => (
                    <Pressable
                      key={host.id}
                      accessibilityRole="button"
                      disabled={model.busy}
                      onPress={() => {
                        model.chooseHost(host);
                        setTab('work');
                      }}
                      style={({ pressed }) => [
                        s.listRow,
                        model.host?.id === host.id && s.selected,
                        pressed && s.pressed,
                      ]}
                    >
                      <Text style={s.listTitle}>{host.name}</Text>
                      <Text style={s.small}>
                        {host.authority === 'desktop' ? 'Desktop session' : 'Durable task host'}
                      </Text>
                      <Text selectable style={s.small}>
                        {host.origin}
                      </Text>
                    </Pressable>
                  ))}
                  <Button primary onPress={() => setAdding(true)} disabled={model.busy}>
                    Add computer
                  </Button>
                  {ephemeralStorage ? (
                    <Text style={s.small}>
                      Browser preview stores credentials only in memory. Install the native app for
                      secure saved connections.
                    </Text>
                  ) : (
                    <Text style={s.small}>
                      Credentials are stored in this device’s secure storage. Your provider login
                      remains on the computer.
                    </Text>
                  )}
                </PageScroll>
              )
            ) : null}
            {tab === 'access' ? (
              <AccessScreen
                model={model}
                onPair={() => {
                  setAdding(true);
                  setTab('hosts');
                }}
              />
            ) : null}
          </View>
        </PageNotices.Provider>
        {!keyboardOpen ? (
          <View style={[s.tabs, { paddingBottom: safe.bottom }]}>
            {(
              [
                ['work', 'Work'],
                ['hosts', 'Computers'],
                ['access', 'Access'],
              ] as const
            ).map(([id, title]) => (
              <Pressable
                key={id}
                accessibilityRole="tab"
                accessibilityState={{ selected: tab === id }}
                onPress={() => {
                  setTab(id);
                  if (id !== 'hosts') setAdding(false);
                }}
                style={({ pressed }) => [s.tab, pressed && s.pressed]}
              >
                <Text style={[s.tabText, tab === id && s.activeTab]}>{title}</Text>
              </Pressable>
            ))}
          </View>
        ) : null}
      </View>
    </KeyboardAvoidingView>
  );
}

function Empty({ onAdd }: Readonly<{ onAdd: () => void }>) {
  return (
    <PageScroll contentContainerStyle={[s.page, s.narrow, s.empty]}>
      <Text style={s.hero}>Bring your{'\n'}computer along.</Text>
      <Text style={s.text}>
        Pair with Bruhs to read live conversations, review work and answer approvals while your
        agents stay on your computer.
      </Text>
      <Button onPress={onAdd} primary>
        Pair a computer
      </Button>
    </PageScroll>
  );
}

function WorkScreen({ model, onPair, onChooseConversation, promptReview }: Readonly<{
  model: Companion;
  onPair: () => void;
  onChooseConversation: (id: string) => void;
  promptReview: ReactNode;
}>) {
  const tablet = useWindowDimensions().width >= 768;
  const notices = useContext(PageNotices);
  if (model.phase === 'expired')
    return (
      <PageScroll contentContainerStyle={[s.page, s.narrow, s.empty]}>
        <Text style={s.heading}>Pair again to continue</Text>
        <Text style={s.text}>
          This saved connection no longer has access. Create a new pairing or task access token on
          your computer.
        </Text>
        <Button primary onPress={onPair}>
          Pair a computer
        </Button>
      </PageScroll>
    );
  return (
    <View style={s.columns}>
      {tablet || !model.selection ? (
        <View style={tablet ? s.master : s.grow}>
          <PageNotices.Provider value={tablet && model.selection ? null : notices}>
            <WorkList model={model} onChooseConversation={onChooseConversation} />
          </PageNotices.Provider>
        </View>
      ) : null}
      {tablet || model.selection ? (
        <View style={s.grow}>
          {model.selection ? (
            <WorkDetail key={`${model.host?.id}:${model.selection.id}`} model={model} promptReview={promptReview} />
          ) : (
            <View style={[s.page, s.empty]}>
              <Text style={s.heading}>Choose a conversation</Text>
              <Text style={s.text}>Its transcript and current approvals will appear here.</Text>
            </View>
          )}
        </View>
      ) : null}
    </View>
  );
}

function WorkList({ model, onChooseConversation }: Readonly<{
  model: Companion;
  onChooseConversation: (id: string) => void;
}>) {
  const [creating, setCreating] = useState(false);
  const taskHost = model.host?.authority === 'tasks';
  const tasks = [...(model.tasks?.tasks ?? [])].sort(
    (a, b) => b.updated_at - a.updated_at || a.id.localeCompare(b.id),
  );
  return (
    <PageScroll contentContainerStyle={{ paddingBottom: 24 }} keyboardShouldPersistTaps="handled">
      <View style={s.page}>
        <Text style={s.heading}>{taskHost ? 'Task workspaces' : 'Desktop threads'}</Text>
        <View style={s.row}>
          <Button onPress={() => void model.refresh()} quiet disabled={model.busy}>
            Refresh
          </Button>
          {taskHost && model.tasks?.can_control ? (
            <Button onPress={() => setCreating((value) => !value)} primary disabled={model.busy}>
              New task
            </Button>
          ) : null}
        </View>
        {creating ? <CreateTask model={model} onDone={() => setCreating(false)} /> : null}
      </View>
      {taskHost ? (
        tasks.length ? (
          tasks.map((task) => (
            <Pressable
              key={task.id}
              accessibilityRole="button"
              accessibilityState={{ selected: task.id === model.selection?.id }}
              onPress={() => onChooseConversation(task.id)}
              style={({ pressed }) => [
                s.listRow,
                task.id === model.selection?.id && s.selected,
                pressed && s.pressed,
              ]}
            >
              <Text style={s.listTitle}>{task.title}</Text>
              <View style={s.row}>
                <StateBadge value={task.state} />
                {task.parent_id ? <Text style={s.small}>Child task</Text> : null}
              </View>
              <Text numberOfLines={1} style={s.small}>
                {task.model}
              </Text>
            </Pressable>
          ))
        ) : (
          <View style={s.page}>
            <Text style={s.text}>
              {model.phase === 'connecting'
                ? 'Connecting to task host…'
                : 'No tasks are shared with this connection yet.'}
            </Text>
          </View>
        )
      ) : model.threads.length ? (
        model.threads.map((thread) => (
          <Pressable
            key={thread.id}
            accessibilityRole="button"
            accessibilityState={{ selected: thread.id === model.selection?.id }}
            onPress={() => onChooseConversation(thread.id)}
            style={({ pressed }) => [
              s.listRow,
              thread.id === model.selection?.id && s.selected,
              pressed && s.pressed,
            ]}
          >
            <Text style={s.listTitle}>{thread.title || 'Untitled thread'}</Text>
            <Text numberOfLines={1} style={s.small}>
              {thread.project}
            </Text>
            <StateBadge value={thread.needs_answer ? 'needs_answer' : thread.state} />
          </Pressable>
        ))
      ) : (
        <View style={s.page}>
          <Text style={s.text}>
            {model.phase === 'connecting'
              ? 'Connecting to your computer…'
              : 'No project threads are visible. Open a project conversation on the computer, then refresh.'}
          </Text>
        </View>
      )}
      {!taskHost ? (
        <View style={s.page}>
          {model.threadOffset ? (
            <Button onPress={() => model.setOffset(0)}>Latest threads</Button>
          ) : null}
          {model.nextOffset !== null ? (
            <Button onPress={() => model.setOffset(model.nextOffset ?? 0)}>More threads</Button>
          ) : null}
        </View>
      ) : null}
    </PageScroll>
  );
}

function WorkDetail({ model, promptReview }: Readonly<{ model: Companion; promptReview: ReactNode }>) {
  const scroll = useRef<ScrollViewInstance>(null),
    composer = useRef<TextInputInstance>(null),
    followComposer = useRef(true),
    follow = useRef(true),
    bodyStart = useRef(0);
  const [composerFocused, setComposerFocused] = useState(false);
  const showPromptReview = Boolean(promptReview);
  useEffect(() => {
    if (!showPromptReview) return;
    const frame = requestAnimationFrame(() => scroll.current?.scrollToEnd({ animated: false }));
    return () => cancelAnimationFrame(frame);
  }, [showPromptReview]);
  const revealComposer = () => {
    if (followComposer.current && composer.current?.isFocused())
      scroll.current?.scrollToEnd({ animated: false });
  };
  const task = model.tasks?.tasks.find((value) => value.id === model.selection?.id);
  const desktop = model.host?.authority === 'desktop';
  const thread = model.thread;
  const permission = thread?.permission;
  const question = thread?.question;
  const questionDraftKey = question && thread && model.host
    ? JSON.stringify([model.host.id, thread.thread.id, question.turn_generation, question.request_id, question.review_hash])
    : null;
  const online = model.phase === 'online';
  const mutable = online && !model.busy && !model.pending && !model.taskPending;
  const ownsTasks = model.tasks?.can_control === true;
  const messageAllowed = desktop
    ? model.session?.device.scopes.includes('prompt')
    : ownsTasks
      ? !task || !running(task.state)
      : true;
  const title = desktop ? thread?.thread.title : task?.title;
  const selectedText = model.textSelection;
  if (selectedText)
    return (
      <PageScroll contentContainerStyle={s.page}>
        <Button onPress={() => model.showText(null)} quiet>
          ← Conversation
        </Button>
        <Text style={s.heading}>
          {selectedText.kind === 'question-link' ? selectedText.title : selectedText.kind === 'diff' ? 'Working tree diff' : 'Full message'}
        </Text>
        {model.textPage ? (
          <>
            <Text selectable style={s.code}>
              {model.textPage.text || (selectedText.kind === 'question-link' ? 'This file is empty.' : 'No changes.')}
            </Text>
            {model.textPage.source_truncated ? (
              <Notice>
                This result reached the computer’s limit. Review the remaining content on the
                computer.
              </Notice>
            ) : null}
            <View style={s.row}>
              <Button
                onPress={() => model.showText({ ...selectedText, offset: 0, revision: undefined })}
              >
                Reload first page
              </Button>
              {model.textPage.next_offset !== null ? (
                <Button
                  onPress={() =>
                    model.showText({
                      ...selectedText,
                      offset: model.textPage?.next_offset ?? 0,
                      revision: model.textPage?.revision,
                    })
                  }
                >
                  Next page
                </Button>
              ) : null}
            </View>
          </>
        ) : (
          <ActivityIndicator color={color.accent} accessibilityLabel="Loading text page" />
        )}
      </PageScroll>
    );
  return (
    <View style={s.grow}>
      {question || permission || task?.approvals.length ? (
        <Button
          onPress={() => {
            followComposer.current = false;
            Keyboard.dismiss();
            follow.current = false;
            scroll.current?.scrollTo({ y: bodyStart.current, animated: false });
          }}
          quiet
        >
          {question ? 'Answer question ↑' : 'Review approval ↑'}
        </Button>
      ) : null}
      <PageScroll
        ref={scroll}
        keyboardShouldPersistTaps="handled"
        keyboardDismissMode="none"
        // Android's caret scrolling would hide Send after our composer scroll.
        // Keep native focus scrolling for the separate task-actions field.
        scrollsChildToFocus={!composerFocused}
        onKeyboardDidShow={() => {
          // Reopening a retained input does not always emit another focus event.
          if (composer.current?.isFocused()) followComposer.current = true;
          revealComposer();
        }}
        onScrollBeginDrag={() => {
          followComposer.current = false;
          follow.current = false;
        }}
        scrollEventThrottle={100}
        onScroll={(event) => {
          const { contentOffset, contentSize, layoutMeasurement } = event.nativeEvent;
          follow.current = contentSize.height - contentOffset.y - layoutMeasurement.height <= 44;
        }}
        onContentSizeChange={() => {
          if (
            (followComposer.current && composer.current?.isFocused()) ||
            (follow.current && !question && !permission && !task?.approvals.length)
          )
            scroll.current?.scrollToEnd({ animated: false });
        }}
        style={s.grow}
        contentContainerStyle={{ flexGrow: 1 }}
        onLayout={revealComposer}
      >
        <View style={[s.page, { gap: 8, paddingBottom: 12 }]}>
          <View style={[s.row, s.between]}>
            <Button onPress={() => model.chooseItem(null)} quiet>
              ← Work
            </Button>
            <Button
              quiet
              onPress={() => {
                followComposer.current = true;
                follow.current = true;
                scroll.current?.scrollToEnd({ animated: false });
              }}
            >
              Message ↓
            </Button>
            {desktop && model.session?.device.scopes.includes('diff') ? (
              <Button onPress={() => model.showText({ kind: 'diff', offset: 0 })} quiet>
                Review diff
              </Button>
            ) : null}
          </View>
          <Text style={[s.subheading, { marginBottom: 0 }]}>
            {title ?? 'Loading conversation…'}
          </Text>
          {task ? <StateBadge value={task.state} /> : null}
          {thread ? <StateBadge value={thread.thread.needs_answer ? 'needs_answer' : thread.thread.state} /> : null}
        </View>
        <View
          style={[s.page, { flexGrow: 1 }]}
          onLayout={(event) => {
            bodyStart.current = event.nativeEvent.layout.y;
          }}
        >
          {question && questionDraftKey ? (
            <QuestionCard key={questionDraftKey} request={question} draftKey={questionDraftKey} drafts={model.questionDrafts}
              canSend={Boolean(messageAllowed)} disabled={!mutable} submit={reply => void model.answerQuestion(question, reply)}
              stageFile={(position, id, file) => model.stageQuestionFile(question, position, id, file)}
              canOpenProject={online && Boolean(model.session?.device.scopes.includes('diff'))}
              previewFile={(id, title) => model.showText({
                kind: 'question-link', title, offset: 0,
                questionLink: { id, requestId: question.request_id, reviewHash: question.review_hash, turnGeneration: question.turn_generation },
              })} />
          ) : null}
          {permission ? (
            <View style={s.permission}>
              <Text style={s.subheading}>{permission.tool}</Text>
              <Text selectable style={s.code}>
                {permission.input}
              </Text>
              {!permission.can_allow ? (
                <Notice>
                  The complete review is too large for the phone. Allow it on the computer, or deny
                  it here.
                </Notice>
              ) : null}
              <View style={s.row}>
                {permission.can_allow ? (
                  <Button
                    onPress={() => void model.decide(permission, 'allow')}
                    primary
                    disabled={!mutable || !model.session?.device.scopes.includes('approve')}
                  >
                    Allow once
                  </Button>
                ) : null}
                <Button
                  onPress={() => void model.decide(permission, 'deny')}
                  danger
                  disabled={!mutable || !model.session?.device.scopes.includes('approve')}
                >
                  Deny
                </Button>
              </View>
              <Text style={s.small}>
                This decision applies only to the displayed request and turn.
              </Text>
            </View>
          ) : null}
          {task?.approvals.map((approval) => (
            <View key={approval.id} style={s.permission}>
              <Text style={s.subheading}>{approval.tool}</Text>
              <Text selectable style={s.code}>
                {JSON.stringify(approval.input, null, 2)}
              </Text>
              <View style={s.row}>
                <Button
                  onPress={() =>
                    void model.taskCommand(
                      { action: 'approval', id: task.id, request_id: approval.id, allow: true },
                      'Approval',
                    )
                  }
                  primary
                  disabled={!mutable || !ownsTasks}
                >
                  Allow once
                </Button>
                <Button
                  onPress={() =>
                    void model.taskCommand(
                      { action: 'approval', id: task.id, request_id: approval.id, allow: false },
                      'Denial',
                    )
                  }
                  danger
                  disabled={!mutable || !ownsTasks}
                >
                  Deny
                </Button>
              </View>
            </View>
          ))}
          {thread?.laptop_action_required && !question ? (
            <Notice>
              A private interaction needs you on the computer. Credential forms are never sent to
              this device.
            </Notice>
          ) : null}
          {task?.error ? <Notice error>{task.error}</Notice> : null}
          {desktop
            ? thread?.messages.map((message) => (
                <View key={`${message.index}:${message.reference}`} style={s.transcript}>
                  <Text style={s.label}>
                    {message.role === 'user'
                      ? 'YOU'
                      : message.streaming
                        ? 'AGENT · LIVE'
                        : message.role.toUpperCase()}
                  </Text>
                  <Text selectable style={s.text}>
                    {message.text}
                  </Text>
                  {message.byte_offset ||
                  message.full_bytes > new TextEncoder().encode(message.text).byteLength ? (
                    <Button
                      quiet
                      onPress={() =>
                        model.showText({
                          kind: 'message',
                          offset: 0,
                          message: { index: message.index, reference: message.reference },
                        })
                      }
                    >
                      Read full message
                    </Button>
                  ) : null}
                </View>
              ))
            : task?.messages.map((message, index) => (
                <View key={`${index}:${message.at}`} style={s.transcript}>
                  <Text style={s.label}>{message.role === 'user' ? 'YOU' : 'AGENT'}</Text>
                  <Text selectable style={s.text}>
                    {message.text}
                  </Text>
                </View>
              ))}
          {task?.live_output ? (
            <View style={s.transcript}>
              <Text style={s.label}>AGENT · LIVE</Text>
              <Text selectable style={s.text}>
                {task.live_output}
              </Text>
            </View>
          ) : null}
          {thread?.next_before !== null && thread?.next_before !== undefined ? (
            <Button
              onPress={() =>
                model.chooseItem({ id: thread.thread.id, before: thread.next_before ?? undefined })
              }
            >
              Earlier messages
            </Button>
          ) : null}
          {model.selection?.before !== undefined ? (
            <Button onPress={() => model.chooseItem({ id: model.selection?.id ?? '' })}>
              Latest messages
            </Button>
          ) : null}
          {task && ownsTasks ? <TaskActions task={task} model={model} enabled={mutable} /> : null}
          {model.delivery ? <Notice>{model.delivery}</Notice> : null}
          {model.pending ? (
            <Button onPress={() => void model.retry()} disabled={!online || model.busy}>
              Check exact saved command
            </Button>
          ) : null}
          {model.taskPending ? (
            <View style={s.permission}>
              <Notice>
                {model.taskPending.description} may have reached the host. Review its transcript and
                current files before continuing. This action will not replay.
              </Notice>
              <Button onPress={() => void model.reviewedTask()} disabled={!online || model.busy}>
                I reviewed the host state
              </Button>
            </View>
          ) : null}
        </View>
        <View style={s.composer}>
          {promptReview}
          <View style={s.row}>
            <Dictation value={model.draft} onText={model.setDraft} />
          </View>
          {desktop && !messageAllowed ? (
            <Text style={s.small}>
              This connection has read access. Grant prompt access on the computer to send messages.
            </Text>
          ) : null}
          <View style={{ flexDirection: 'row', alignItems: 'flex-end', gap: 10 }}>
            <View style={{ flex: 1, minWidth: 0 }}>
              <Field
                label={desktop || ownsTasks ? 'Message your agent' : 'Propose a message to the owner'}
                value={model.draft}
                onChange={model.setDraft}
                multiline
                maxLength={32768}
                placeholder="Continue the work…"
                testID="composer"
                inputRef={composer}
                onFocus={() => {
                  followComposer.current = true;
                  setComposerFocused(true);
                  follow.current = true;
                  scroll.current?.scrollToEnd({ animated: false });
                }}
                onBlur={() => {
                  setComposerFocused(false);
                }}
              />
            </View>
            <View style={{ maxWidth: '40%', alignSelf: 'center' }}>
              <Button
                primary
                onPress={() => void model.sendPrompt()}
                disabled={!mutable || !messageAllowed || !model.draft.trim()}
              >
                {desktop || ownsTasks ? 'Send' : 'Propose'}
              </Button>
            </View>
          </View>
        </View>
      </PageScroll>
    </View>
  );
}

function TaskActions({
  task,
  model,
  enabled,
}: Readonly<{ task: Task; model: Companion; enabled: boolean }>) {
  const [expanded, expand] = useState(false),
    [question, setQuestion] = useState('');
  return (
    <View style={s.section}>
      <Button quiet onPress={() => expand((value) => !value)}>
        {expanded ? 'Hide task actions ↑' : 'Task actions ↓'}
      </Button>
      {expanded ? (
        <>
          <Text style={s.small}>
            {task.model}
            {task.project_path ? ` · ${task.project_path}` : ''}
          </Text>
          <View style={s.row}>
            <Button
              danger
              disabled={!enabled || !running(task.state)}
              onPress={() =>
                void model.taskCommand({ action: 'cancel', id: task.id }, 'Stop task and children')
              }
            >
              Stop task + children
            </Button>
            {task.parent_id ? (
              <Button
                disabled={!enabled || running(task.state)}
                onPress={() =>
                  void model.taskCommand({ action: 'bring_back', id: task.id }, 'Bring back answer')
                }
              >
                Bring back answer
              </Button>
            ) : null}
            <Button
              disabled={!enabled || running(task.state)}
              onPress={() =>
                void model.taskCommand(
                  { action: 'schedule_reset', id: task.id },
                  'Resume at reported reset',
                )
              }
            >
              Resume at reset
            </Button>
          </View>
          <Field
            label="Side question or verification request"
            value={question}
            onChange={setQuestion}
            multiline
            maxLength={32768}
          />
          <View style={s.row}>
            <Button
              disabled={!enabled || !question.trim()}
              onPress={() =>
                void model.taskCommand(
                  {
                    action: 'fork',
                    id: task.id,
                    title: `Side chat: ${task.title}`,
                    prompt: question,
                    relationship: 'side_chat',
                    model: task.model,
                    isolation: { kind: 'shared' },
                  },
                  'Side chat',
                )
              }
            >
              Ask a side question
            </Button>
            <Button
              disabled={!enabled}
              onPress={() =>
                void model.taskCommand(
                  {
                    action: 'fork',
                    id: task.id,
                    title: `Verify: ${task.title}`,
                    prompt:
                      question.trim() ||
                      'Verify the implementation against the attached specification and cite evidence.',
                    relationship: 'verify',
                    model: task.model,
                    isolation: { kind: 'shared' },
                  },
                  'Verification',
                )
              }
            >
              Verify work
            </Button>
          </View>
        </>
      ) : null}
    </View>
  );
}

function CreateTask({ model, onDone }: Readonly<{ model: Companion; onDone: () => void }>) {
  const first = model.tasks?.tasks[0];
  const [title, setTitle] = useState(''),
    [path, setPath] = useState(first?.project_path ?? ''),
    [runtime, setRuntime] = useState(first?.model ?? 'codex/default'),
    [prompt, setPrompt] = useState(''),
    [isolate, setIsolate] = useState(true),
    [base, setBase] = useState('main'),
    [access, setAccess] = useState('default');
  return (
    <View style={s.section}>
      <Field label="Task title" value={title} onChange={setTitle} maxLength={128} />
      <Field
        label="Absolute project folder on the host"
        value={path}
        onChange={setPath}
        placeholder="/home/me/project"
      />
      <Field label="Runtime / model" value={runtime} onChange={setRuntime} />
      <Field
        label="Task instructions"
        value={prompt}
        onChange={setPrompt}
        multiline
        maxLength={32768}
      />
      <Text style={s.label}>Access mode</Text>
      <View style={s.row}>
        {(
          [
            ['plan', 'Plan'],
            ['default', 'Ask first'],
            ['acceptEdits', 'Accept edits'],
            ['bypassPermissions', 'Full access'],
          ] as const
        ).map(([value, label]) => (
          <Button key={value} primary={access === value} onPress={() => setAccess(value)}>
            {label}
          </Button>
        ))}
      </View>
      <View style={s.row}>
        <Button primary={isolate} onPress={() => setIsolate(true)}>
          New worktree
        </Button>
        <Button primary={!isolate} onPress={() => setIsolate(false)}>
          Current folder
        </Button>
      </View>
      {isolate ? (
        <Field label="Worktree base branch" value={base} onChange={setBase} />
      ) : (
        <Text style={s.small}>
          Edits share this folder with other work. Finish other writers before using it.
        </Text>
      )}
      <Button
        primary
        disabled={
          model.busy ||
          !!model.taskPending ||
          !title.trim() ||
          !path.trim() ||
          !runtime.trim() ||
          !prompt.trim()
        }
        onPress={() =>
          void model.taskCommand(
            {
              action: 'create',
              task: {
                title,
                project_path: path,
                model: runtime,
                prompt,
                permission_mode: access,
                isolation: isolate ? { kind: 'worktree', base_branch: base } : { kind: 'shared' },
                spec_id: null,
              },
            },
            'Create task',
            onDone,
          )
        }
      >
        Start on host
      </Button>
    </View>
  );
}

function AccessScreen({ model, onPair }: Readonly<{ model: Companion; onPair: () => void }>) {
  const host = model.host;
  if (!host)
    return (
      <PageScroll contentContainerStyle={[s.page, s.narrow]}>
        <Text style={s.heading}>Device access</Text>
        <Text style={s.text}>
          Pair a computer first. Choose read, prompt, approval and diff permissions on that
          computer.
        </Text>
        <Button primary onPress={onPair}>
          Pair a computer
        </Button>
      </PageScroll>
    );
  return (
    <PageScroll contentContainerStyle={[s.page, s.narrow]}>
      <Text style={s.heading}>Access to {host.name}</Text>
      <Text selectable style={s.text}>
        {host.origin}
      </Text>
      <Text style={s.small}>
        {host.authority === 'desktop' ? 'Desktop-owned session' : 'Server-owned task workspaces'} ·
        credentials remain scoped to this authority.
      </Text>
      {model.session ? (
        <>
          <Text style={s.subheading}>Granted on the computer</Text>
          <View style={s.row}>
            {model.session.device.scopes.map((scope) => (
              <StateBadge key={scope} value={scope === 'linked_drafts' ? 'Open linked drafts' : scope} />
            ))}
          </View>
          <Text style={s.text}>
            Expires {new Date(model.session.device.expires_at_ms).toLocaleString()}
          </Text>
          <Text style={s.small}>
            A desktop restart ends this pairing. A replacement approval card requires a new
            decision.
          </Text>
        </>
      ) : model.tasks ? (
        <Text style={s.text}>
          {model.tasks.can_control
            ? 'Owner task controls are enabled.'
            : 'Guest view. Your messages require owner review.'}
        </Text>
      ) : null}
      <View style={s.section}>
        <Text style={s.text}>
          Providers, credentials, recordings and life data stay on the computer. Task and chat text
          may itself contain private project information.
        </Text>
        <Text style={s.small}>
          {ephemeralStorage
            ? 'This browser preview retains tokens only until the page closes.'
            : 'Tokens and exact pending command receipts use encrypted OS secure storage, excluded from Android backup and pinned to this device on iOS.'}
        </Text>
      </View>
      <Button
        disabled={model.busy}
        danger
        onPress={() =>
          Alert.alert(
            host.authority === 'desktop' ? 'Disconnect this device?' : 'Remove saved task host?',
            host.authority === 'desktop'
              ? 'This revokes this pairing on the computer and removes its saved credential.'
              : 'This removes the saved token from this phone. Revoke the grant in Workbench on the host to end that token’s access.',
            [
              { text: 'Cancel', style: 'cancel' },
              { text: 'Remove', style: 'destructive', onPress: () => void model.removeHost(host) },
            ],
          )
        }
      >
        {host.authority === 'desktop' ? 'Revoke pairing and remove' : 'Remove saved task host'}
      </Button>
      <Button onPress={onPair} disabled={model.busy}>
        Pair another computer
      </Button>
    </PageScroll>
  );
}
