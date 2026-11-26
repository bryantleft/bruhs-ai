import { useCallback, useEffect, useRef, useState } from 'react';
import { AppState } from 'react-native';
import * as Crypto from 'expo-crypto';
import { HostClient, pairDesktop, type TextPageRequest } from './client';
import { transport } from './transport';
import { vault } from './storage';
import {
  type Failure,
  type Host,
  type LinkedDraftChoice,
  type LinkedDraftTarget,
  type NativeAction,
  type NativeCommand,
  type Pairing,
  type Permission,
  type PendingQuestion,
  type QuestionReply,
  type Receipt,
  type Session,
  type TaskSnapshot,
  type TextPage,
  type Thread,
  type ThreadSnapshot,
  acknowledgeDraft,
  draftKey,
  failure,
  messageOf,
  nextCommand,
  originOf,
} from './domain';
import { canMutate, deliveryStatus, recoverDelivery } from './delivery';
import type { UncertainTask } from './vault';
import type { QuestionDraft } from './QuestionCard';
import type { PickedQuestionFile } from './questionFiles';

// Matches MobileRead's existing 1000ms coalesced snapshot cadence. Only one
// refresh is in flight, and background apps stop making requests.
const REFRESH_MS = 1000;
export type Phase = 'none' | 'connecting' | 'online' | 'offline' | 'expired';
export type Selection = Readonly<{ id: string; before?: number }>;
export function useCompanion() {
  const [hosts, setHosts] = useState<readonly Host[]>([]);
  const [host, setHost] = useState<Host | null>(null);
  const [phase, setPhase] = useState<Phase>('none');
  const [problem, setProblem] = useState('');
  const [busy, setBusy] = useState(false);
  const [session, setSession] = useState<Session | null>(null);
  const [threads, setThreads] = useState<readonly Thread[]>([]);
  const [threadOffset, setThreadOffset] = useState(0);
  const [nextOffset, setNextOffset] = useState<number | null>(null);
  const [selection, setSelection] = useState<Selection | null>(null);
  const [thread, setThread] = useState<ThreadSnapshot | null>(null);
  const [tasks, setTasks] = useState<TaskSnapshot | null>(null);
  const [textSelection, setTextSelection] = useState<TextPageRequest | null>(null);
  const [textPage, setTextPage] = useState<TextPage | null>(null);
  const [drafts, setDrafts] = useState<Readonly<Record<string, string>>>({});
  const draftsRef = useRef(drafts),
    draftHosts = useRef(new Set<string>());
  const questionDrafts = useRef(new Map<string, QuestionDraft>());
  const [pending, setPending] = useState<NativeCommand | null>(null);
  const [taskPending, setTaskPending] = useState<UncertainTask | null>(null);
  const [delivery, setDelivery] = useState('');
  const current = useRef({ host, selection, threadOffset, textSelection, phase });
  current.current = { host, selection, threadOffset, textSelection, phase };
  const pendingRef = useRef<NativeCommand | null>(null);
  const loaded = useRef<string | null>(null);
  const epoch = useRef(0);
  const foreground = useRef(AppState.currentState === 'active');
  const refreshing = useRef(false);
  const mutation = useRef(false);
  const sessionRef = useRef<Session | null>(null);
  const readAbort = useRef<AbortController | null>(null);
  const problemKind = useRef<Failure['kind'] | null>(null);
  const loadedText = useRef<number | null>(null);

  const updateDrafts = (
    update: (previous: Readonly<Record<string, string>>) => Readonly<Record<string, string>>,
  ): void => {
    draftsRef.current = update(draftsRef.current);
    setDrafts(draftsRef.current);
  };
  const saveDrafts = async (active: Host): Promise<void> => {
    if (!draftHosts.current.has(active.id)) return;
    const prefix = `${active.id}:${active.authority}:`;
    const values = Object.fromEntries(
      Object.entries(draftsRef.current)
        .filter(([key, text]) => key.startsWith(prefix) && text.length > 0)
        .map(([key, text]) => [key.slice(prefix.length), text]),
    );
    const result = await vault.saveDrafts(active, values);
    if (!result.ok && current.current.host?.id === active.id) {
      problemKind.current = 'storage';
      setProblem(result.error.message);
    }
  };

  const replacePending = (command: NativeCommand | null) => {
    pendingRef.current = command;
    setPending(command);
  };
  const belongs = (active: Host, generation: number): boolean =>
    foreground.current && current.current.host?.id === active.id && epoch.current === generation;
  const report = (error: Failure) => {
    problemKind.current = error.kind;
    setProblem(error.message);
    if (error.kind === 'unauthorized') {
      questionDrafts.current.clear();
      setPhase('expired');
      setThread(null);
      setTasks(null);
      setThreads([]);
      setTextPage(null);
      const active = current.current.host;
      if (active) {
        updateDrafts((previous) =>
          Object.fromEntries(
            Object.entries(previous).filter(([key]) => !key.startsWith(`${active.id}:`)),
          ),
        );
        void saveDrafts(active);
      }
    }
  };

  useEffect(() => {
    let live = true;
    void vault.hosts().then((result) => {
      if (!live) return;
      if (result.ok) setHosts(result.value);
      else report(result.error);
    });
    return () => {
      live = false;
    };
  }, []);

  const applyReceipt = useCallback(async (active: Host, receipt: Receipt): Promise<void> => {
    const command = pendingRef.current;
    if (!command || command.id !== receipt.id || command.sequence !== receipt.sequence) {
      report({
        kind: 'conflict',
        message:
          'The host returned a receipt for a different command. The saved command remains locked until its exact receipt is found.',
      });
      return;
    }
    const linked = command.action.kind.endsWith('linked_draft');
    setDelivery(linked && receipt.outcome.status === 'accepted' ? '' : deliveryStatus(receipt));
    if (receipt.outcome.status === 'pending') return;
    const saved = await vault.savePending(active, null);
    if (current.current.host?.id !== active.id) return;
    if (!saved.ok) {
      report(saved.error);
      return;
    }
    const previousSession = sessionRef.current;
    if (previousSession) {
      const updated = {
        ...previousSession,
        next_sequence: Math.max(previousSession.next_sequence, receipt.sequence + 1),
        last_command: receipt,
      };
      sessionRef.current = updated;
      setSession(updated);
    }
    if (command?.action.kind === 'prompt') {
      const action = command.action,
        key = draftKey(active, action.thread_id);
      updateDrafts((previous) => ({
        ...previous,
        [key]: acknowledgeDraft(previous[key] ?? '', action.text, receipt),
      }));
      await saveDrafts(active);
    }
    replacePending(null);
  }, []);

  const refresh = useCallback(async (): Promise<void> => {
    const {
      host: active,
      selection: item,
      threadOffset: offset,
      textSelection: page,
      phase: state,
    } = current.current;
    if (!active || !foreground.current || refreshing.current || state === 'expired') return;
    refreshing.current = true;
    const generation = epoch.current;
    const abort = new AbortController();
    readAbort.current = abort;
    const client = new HostClient(active, transport, abort.signal);
    try {
      if (!draftHosts.current.has(active.id)) {
        const savedDrafts = await vault.drafts(active);
        if (!belongs(active, generation)) return;
        if (!savedDrafts.ok) {
          report(savedDrafts.error);
          setPhase('offline');
          return;
        }
        draftHosts.current.add(active.id);
        updateDrafts((previous) => ({
          ...Object.fromEntries(
            Object.entries(savedDrafts.value ?? {}).map(([id, text]) => [
              draftKey(active, id),
              text,
            ]),
          ),
          ...previous,
        }));
      }
      if (active.authority === 'desktop') {
        const info = await client.session();
        if (!belongs(active, generation)) return;
        if (!info.ok) {
          report(info.error);
          if (info.error.kind !== 'unauthorized') setPhase('offline');
          return;
        }
        if (info.value.device.revoked || info.value.device.expires_at_ms <= Date.now()) {
          report({
            kind: 'unauthorized',
            message: 'This pairing expired or was revoked. Pair again from the computer.',
          });
          return;
        }
        sessionRef.current = info.value;
        setSession(info.value);
        if ((page?.kind === 'diff' || page?.kind === 'question-link') && !info.value.device.scopes.includes('diff')) {
          current.current.textSelection = null;
          setTextSelection(null);
          setTextPage(null);
          report({ kind: 'forbidden', message: 'Project preview access ended on the computer.' });
          setPhase('online');
          return;
        }
        if (loaded.current !== active.id) {
          const saved = await vault.pending(active);
          if (!belongs(active, generation)) return;
          if (!saved.ok) {
            report(saved.error);
            setPhase('offline');
            return;
          }
          replacePending(saved.value);
          loaded.current = active.id;
        }
        const recovery = recoverDelivery(pendingRef.current, info.value);
        if (recovery.kind === 'receipt' && pendingRef.current)
          await applyReceipt(active, recovery.receipt);
        else if (recovery.kind === 'review') setDelivery(recovery.message);
        else if (recovery.kind === 'retry')
          setDelivery(
            'A saved command needs a delivery check. Retrying sends its exact original UUID, sequence and payload.',
          );
        if (item && page && loadedText.current !== generation) {
          const result = await client.text(item.id, page);
          if (!belongs(active, generation)) return;
          loadedText.current = generation;
          if (!result.ok) {
            report(result.error);
            if (result.error.kind !== 'unauthorized') setPhase('online');
            return;
          }
          setTextPage(result.value.value);
        } else if (item && page) {
          // A paged diff/message is pinned until the user reloads it. Stream
          // invalidations still refresh session expiry and command receipts.
        } else if (item) {
          const result = await client.thread(item.id, item.before);
          if (!belongs(active, generation)) return;
          if (!result.ok) {
            report(result.error);
            if (result.error.kind !== 'unauthorized') setPhase('offline');
            return;
          }
          setThread(result.value.value);
        } else {
          const result = await client.threads(offset);
          if (!belongs(active, generation)) return;
          if (!result.ok) {
            report(result.error);
            if (result.error.kind !== 'unauthorized') setPhase('offline');
            return;
          }
          setThreads(result.value.value.threads);
          setNextOffset(result.value.value.next_offset);
        }
      } else {
        if (loaded.current !== active.id) {
          const saved = await vault.taskPending(active);
          if (!belongs(active, generation)) return;
          if (!saved.ok) {
            report(saved.error);
            setPhase('offline');
            return;
          }
          setTaskPending(saved.value);
          loaded.current = active.id;
        }
        const result = await client.tasks();
        if (!belongs(active, generation)) return;
        if (!result.ok) {
          report(
            result.error.kind === 'forbidden'
              ? {
                  kind: 'unauthorized',
                  message: 'This task access was rejected. Pair with a new token from the host.',
                }
              : result.error,
          );
          if (result.error.kind !== 'forbidden' && result.error.kind !== 'unauthorized')
            setPhase('offline');
          return;
        }
        setTasks(result.value);
      }
      if (belongs(active, generation)) {
        setPhase('online');
        if (problemKind.current === 'network') {
          problemKind.current = null;
          setProblem('');
        }
      }
    } catch (error) {
      if (belongs(active, generation)) {
        setProblem(messageOf(error));
        setPhase('offline');
      }
    } finally {
      refreshing.current = false;
      if (readAbort.current === abort) readAbort.current = null;
    }
  }, [applyReceipt]);

  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), REFRESH_MS);
    const listener = AppState.addEventListener('change', (state) => {
      foreground.current = state === 'active';
      if (!foreground.current) {
        readAbort.current?.abort();
        if (current.current.host) void saveDrafts(current.current.host);
        setPhase((previous) => (previous === 'online' ? 'offline' : previous));
      } else void refresh();
    });
    return () => {
      clearInterval(timer);
      listener.remove();
      readAbort.current?.abort();
    };
  }, [refresh]);

  useEffect(() => {
    if (!host) return;
    const timer = setTimeout(() => void saveDrafts(host), REFRESH_MS);
    return () => clearTimeout(timer);
  }, [drafts, host]);

  const chooseHost = useCallback(
    (next: Host | null): void => {
      if (current.current.host) void saveDrafts(current.current.host);
      if (current.current.host?.id !== next?.id) questionDrafts.current.clear();
      epoch.current++;
      readAbort.current?.abort();
      loaded.current = null;
      sessionRef.current = null;
      replacePending(null);
      current.current = {
        host: next,
        selection: null,
        threadOffset: 0,
        textSelection: null,
        phase: next ? 'connecting' : 'none',
      };
      setHost(next);
      setSelection(null);
      setThread(null);
      setTasks(null);
      setThreads([]);
      setTextPage(null);
      setTextSelection(null);
      setThreadOffset(0);
      setNextOffset(null);
      setSession(null);
      setTaskPending(null);
      setDelivery('');
      setProblem('');
      setPhase(next ? 'connecting' : 'none');
      void refresh();
    },
    [refresh],
  );

  const chooseItem = useCallback((next: Selection | null): void => {
    epoch.current++;
    readAbort.current?.abort();
    current.current.selection = next;
    current.current.textSelection = null;
    setSelection(next);
    setThread(null);
    setTextSelection(null);
    setTextPage(null);
    setProblem('');
    void refresh();
  }, [refresh]);
  const showText = (next: TextPageRequest | null): void => {
    epoch.current++;
    readAbort.current?.abort();
    current.current.textSelection = next;
    setTextSelection(next);
    setTextPage(null);
    setProblem('');
    void refresh();
  };
  const setOffset = (offset: number): void => {
    epoch.current++;
    readAbort.current?.abort();
    current.current.threadOffset = offset;
    setThreadOffset(offset);
    void refresh();
  };

  async function addDesktop(pairing: Pairing, name: string): Promise<boolean> {
    if (mutation.current) return false;
    mutation.current = true;
    setBusy(true);
    setProblem('');
    try {
      const result = await pairDesktop(transport, pairing, name.trim(), __DEV__);
      if (!result.ok) {
        report(result.error);
        return false;
      }
      const next: Host = {
        id: Crypto.randomUUID(),
        name: name.trim(),
        origin: pairing.origin,
        authority: 'desktop',
        token: result.value.token,
        expiresAt: result.value.device.expires_at_ms,
      };
      const saved = await vault.saveHost(next);
      if (!saved.ok) {
        await new HostClient(next, transport).disconnect();
        report(saved.error);
        return false;
      }
      setHosts((previous) => [...previous, next]);
      chooseHost(next);
      return true;
    } finally {
      mutation.current = false;
      setBusy(false);
    }
  }

  async function addTasks(origin: string, name: string, token: string): Promise<boolean> {
    const parsed = originOf(origin, __DEV__);
    if (!parsed.ok) {
      report(parsed.error);
      return false;
    }
    if (!name.trim() || !token.trim()) {
      setProblem('Enter a host name and the task token issued on your computer.');
      return false;
    }
    if (mutation.current) return false;
    mutation.current = true;
    setBusy(true);
    setProblem('');
    try {
      const next: Host = {
        id: Crypto.randomUUID(),
        name: name.trim(),
        origin: parsed.value,
        authority: 'tasks',
        token: token.trim(),
      };
      const result = await new HostClient(next, transport).tasks();
      if (!result.ok) {
        report(result.error);
        return false;
      }
      const saved = await vault.saveHost(next);
      if (!saved.ok) {
        report(saved.error);
        return false;
      }
      setHosts((previous) => [...previous, next]);
      chooseHost(next);
      return true;
    } finally {
      mutation.current = false;
      setBusy(false);
    }
  }

  async function removeHost(active: Host): Promise<void> {
    if (mutation.current) return;
    mutation.current = true;
    setBusy(true);
    try {
      if (active.authority === 'desktop') {
        const revoked = await new HostClient(active, transport).disconnect();
        if (!revoked.ok && revoked.error.kind !== 'unauthorized') {
          report(revoked.error);
          return;
        }
      }
      const result = await vault.removeHost(active.id);
      if (!result.ok) {
        report(result.error);
        return;
      }
      setHosts((previous) => previous.filter((value) => value.id !== active.id));
      draftHosts.current.delete(active.id);
      updateDrafts((previous) =>
        Object.fromEntries(
          Object.entries(previous).filter(([key]) => !key.startsWith(`${active.id}:`)),
        ),
      );
      if (current.current.host?.id === active.id) chooseHost(null);
    } finally {
      mutation.current = false;
      setBusy(false);
    }
  }

  async function postNative(active: Host, command: NativeCommand): Promise<void> {
    const result = await new HostClient(active, transport).command(command);
    if (current.current.host?.id !== active.id) return;
    if (result.ok) await applyReceipt(active, result.value);
    else {
      report(result.error);
      setDelivery('Delivery may be pending. Check the exact saved command before sending another.');
      if (result.error.kind !== 'unauthorized') setPhase('offline');
    }
    await refresh();
  }
  async function sendNative(action: NativeAction, onSaved?: (id: string) => void, targetHost?: string): Promise<void> {
    const active = current.current.host,
      info = sessionRef.current;
    if (
      !active ||
      (targetHost && active.id !== targetHost) ||
      !info ||
      mutation.current ||
      !canMutate(
        active,
        current.current.phase === 'online',
        pendingRef.current,
        info,
        action.kind === 'prompt' || action.kind === 'answer_question'
          ? 'prompt'
          : action.kind === 'permission' ? 'approve' : 'linked_drafts',
      )
    )
      return;
    const command = nextCommand(Crypto.randomUUID(), info.next_sequence, action);
    if (!command.ok) {
      report(command.error);
      return;
    }
    if (new TextEncoder().encode(JSON.stringify(command.value)).byteLength > 64 * 1024) {
      setProblem('Shorten this command to fit the computer’s 64 KiB command budget.');
      return;
    }
    mutation.current = true;
    setBusy(true);
    try {
      const saved = await vault.savePending(active, command.value);
      if (!saved.ok) {
        report(saved.error);
        return;
      }
      if (current.current.host?.id !== active.id) return;
      replacePending(command.value);
      onSaved?.(command.value.id);
      setDelivery(action.kind.endsWith('linked_draft') ? 'Preparing linked draft…' : 'Sending…');
      await postNative(active, command.value);
    } finally {
      mutation.current = false;
      setBusy(false);
    }
  }
  const readLinkedDraft = useCallback((hostId: string, linkId: string, signal: AbortSignal) => {
    const active = current.current.host;
    if (!active || active.id !== hostId || active.authority !== 'desktop') {
      return Promise.resolve(failure('conflict', 'The selected computer changed.'));
    }
    return new HostClient(active, transport).linkedDraft(linkId, signal);
  }, []);
  async function retry(): Promise<void> {
    const active = current.current.host,
      command = pendingRef.current;
    if (!active || !command || mutation.current || current.current.phase !== 'online') return;
    mutation.current = true;
    setBusy(true);
    try {
      await postNative(active, command);
    } finally {
      mutation.current = false;
      setBusy(false);
    }
  }

  async function taskCommand(
    command: unknown,
    description: string,
    onAccepted?: () => void,
  ): Promise<void> {
    const active = current.current.host;
    if (
      !active ||
      active.authority !== 'tasks' ||
      current.current.phase !== 'online' ||
      !tasks?.can_control ||
      taskPending ||
      mutation.current
    )
      return;
    mutation.current = true;
    setBusy(true);
    try {
      const pending = { id: Crypto.randomUUID(), description, at: Date.now() };
      const saved = await vault.saveTaskPending(active, pending);
      if (!saved.ok) {
        report(saved.error);
        return;
      }
      if (current.current.host?.id !== active.id) return;
      setTaskPending(pending);
      const result = await new HostClient(active, transport).taskCommand(command);
      if (current.current.host?.id !== active.id) return;
      if (result.ok || result.error.delivery === 'rejected') {
        const cleared = await vault.saveTaskPending(active, null);
        if (!cleared.ok) {
          report(cleared.error);
          return;
        }
        setTaskPending(null);
      }
      if (result.ok) {
        setTasks(result.value);
        setDelivery(`${description} accepted by the host.`);
        onAccepted?.();
      } else {
        report(result.error);
        if (result.error.delivery !== 'rejected') {
          setDelivery(
            'Delivery is uncertain. This task action will not replay. Reconnect, review the host, then mark it reviewed.',
          );
          if (result.error.kind !== 'unauthorized') setPhase('offline');
        }
      }
    } finally {
      mutation.current = false;
      setBusy(false);
    }
  }

  async function reviewedTask(): Promise<void> {
    const active = current.current.host;
    if (!active || current.current.phase !== 'online') return;
    const result = await vault.saveTaskPending(active, null);
    if (result.ok) {
      setTaskPending(null);
      setDelivery('Host state reviewed. No earlier action was replayed.');
    } else report(result.error);
  }

  const key = host && selection ? draftKey(host, selection.id) : null;
  const draft = key ? (drafts[key] ?? '') : '';
  const setDraft = (text: string): void => {
    if (key) updateDrafts((previous) => ({ ...previous, [key]: text }));
  };
  const placeLinkedDraft = useCallback((hostId: string, threadId: string, expected: string, text: string): boolean => {
    const { host: active, selection: item, phase: state } = current.current;
    if (active?.id !== hostId || item?.id !== threadId || state !== 'online' || !draftHosts.current.has(hostId)) {
      return false;
    }
    const key = draftKey(active, threadId);
    if ((draftsRef.current[key] ?? '') !== expected) return false;
    if (expected !== text) {
      updateDrafts((previous) => ({ ...previous, [key]: text }));
      void saveDrafts(active);
    }
    return true;
  }, []);
  async function sendPrompt(): Promise<void> {
    const active = current.current.host,
      item = current.current.selection,
      text = draft.trim();
    if (!active || !item || !text) return;
    if (new TextEncoder().encode(text).byteLength > 32 * 1024) {
      setProblem('Shorten the prompt to the phone’s 32 KiB message budget.');
      return;
    }
    if (active.authority === 'desktop')
      await sendNative({ kind: 'prompt', thread_id: item.id, text });
    else if (tasks?.can_control) {
      await taskCommand({ action: 'send', id: item.id, prompt: text }, 'Message', () => {
        updateDrafts((previous) => ({
          ...previous,
          [draftKey(active, item.id)]:
            (previous[draftKey(active, item.id)] ?? '').trim() === text
              ? ''
              : (previous[draftKey(active, item.id)] ?? ''),
        }));
        void saveDrafts(active);
      });
    } else {
      if (mutation.current || current.current.phase !== 'online') return;
      mutation.current = true;
      setBusy(true);
      try {
        if (taskPending) return;
        const pending = { id: Crypto.randomUUID(), description: 'Guest proposal', at: Date.now() };
        const saved = await vault.saveTaskPending(active, pending);
        if (!saved.ok) {
          report(saved.error);
          return;
        }
        setTaskPending(pending);
        const result = await new HostClient(active, transport).proposal(text);
        if (current.current.host?.id !== active.id) return;
        if (result.ok || result.error.delivery === 'rejected') {
          const cleared = await vault.saveTaskPending(active, null);
          if (!cleared.ok) {
            report(cleared.error);
            return;
          }
          setTaskPending(null);
        }
        if (result.ok) {
          setDelivery('Sent to the owner for review.');
          updateDrafts((previous) => ({
            ...previous,
            [draftKey(active, item.id)]:
              (previous[draftKey(active, item.id)] ?? '').trim() === text
                ? ''
                : (previous[draftKey(active, item.id)] ?? ''),
          }));
          void saveDrafts(active);
        } else {
          report(result.error);
          if (result.error.delivery !== 'rejected' && result.error.kind !== 'unauthorized')
            setPhase('offline');
        }
      } finally {
        mutation.current = false;
        setBusy(false);
      }
    }
  }
  const decide = (permission: Permission, decision: 'allow' | 'deny'): Promise<void> =>
    selection
      ? sendNative({
          kind: 'permission',
          thread_id: selection.id,
          request_id: permission.request_id,
          turn_generation: permission.turn_generation,
          review_hash: permission.review_hash,
          decision,
        })
      : Promise.resolve();
  const answerQuestion = (question: PendingQuestion, reply: QuestionReply): Promise<void> =>
    host && thread && selection?.id === thread.thread.id && thread.question?.review_hash === question.review_hash
      ? sendNative({
          kind: 'answer_question',
          thread_id: thread.thread.id,
          request_id: question.request_id,
          turn_generation: question.turn_generation,
          review_hash: question.review_hash,
          reply,
        }, undefined, host.id)
      : Promise.resolve();
  async function stageQuestionFile(
    question: PendingQuestion,
    position: number,
    id: string,
    file?: PickedQuestionFile,
  ) {
    const active = current.current;
    if (!host || active.host?.id !== host.id || host.authority !== 'desktop'
      || active.phase !== 'online' || !thread || active.selection?.id !== thread.thread.id
      || thread.question?.request_id !== question.request_id
      || thread.question.turn_generation !== question.turn_generation
      || thread.question.review_hash !== question.review_hash
      || thread.question.responding || !thread.question.can_attach
      || !question.questions[position]?.allowCustom
      || !sessionRef.current?.device.scopes.includes('prompt')
      || mutation.current || pendingRef.current) {
      return failure('conflict', 'Reload the pending question and try again.');
    }
    const result = await new HostClient(host, transport).questionFile(thread.thread.id, question, position, id, file);
    if (!result.ok && result.error.kind === 'unauthorized' && current.current.host?.id === host.id) {
      report(result.error);
    }
    return result;
  }
  return {
    readLinkedDraft,
    openLinkedDraft: (hostId: string, target: LinkedDraftTarget, onSaved: (id: string) => void) =>
      sendNative({ kind: 'open_linked_draft', target }, onSaved, hostId),
    continueLinkedDraft: (hostId: string, id: string, choice: LinkedDraftChoice, onSaved: (id: string) => void) =>
      sendNative({ kind: 'continue_linked_draft', link_id: id, choice }, onSaved, hostId),
    hosts,
    host,
    chooseHost,
    phase,
    problem,
    clearProblem: () => setProblem(''),
    busy,
    session,
    threads,
    nextOffset,
    threadOffset,
    setOffset,
    selection,
    chooseItem,
    thread,
    tasks,
    textSelection,
    textPage,
    showText,
    draft,
    setDraft,
    placeLinkedDraft,
    pending,
    taskPending,
    delivery,
    refresh,
    addDesktop,
    addTasks,
    removeHost,
    retry,
    taskCommand,
    reviewedTask,
    sendPrompt,
    decide,
    answerQuestion,
    stageQuestionFile,
    questionDrafts: questionDrafts.current,
  };
}
