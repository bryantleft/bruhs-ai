import { useEffect, useState } from 'react';
import { AppState } from 'react-native';
import type { LinkedDraftChoice, LinkedDraftStatus } from './domain';
import { type LinkedPrompt, pullRequestUrl } from './promptLink';
import type { useCompanion } from './useCompanion';

type Companion = ReturnType<typeof useCompanion>;
type Request = {
  hostId: string;
  id: string;
  commandId: string;
  link: LinkedPrompt;
};

export function useLinkedDraftTarget(model: Companion, link: LinkedPrompt | undefined) {
  const [request, setRequest] = useState<Request | null>(null);
  const [review, setReview] = useState<LinkedDraftStatus | null>(null);
  const [foreground, setForeground] = useState(AppState.currentState === 'active');
  const hostId = model.host?.id;
  const supported = model.host?.authority === 'desktop'
    && model.session?.device.scopes.includes('linked_drafts') === true;
  const canPrepare = supported && model.phase === 'online' && !model.pending && !model.busy;
  const active = request?.link === link && request?.hostId === hostId ? request : null;
  const status = active ? review : null;
  const online = model.phase === 'online';
  const pending = model.pending;
  const receipt = model.session?.last_command;
  const read = model.readLinkedDraft;

  useEffect(() => {
    const subscription = AppState.addEventListener('change', state => setForeground(state === 'active'));
    return () => subscription.remove();
  }, []);

  useEffect(() => {
    if (!active || !online || !foreground || pending) return;
    if (receipt?.id === active.commandId
      && (receipt.outcome.status === 'rejected' || receipt.outcome.status === 'unconfirmed')) {
      const message = receipt.outcome.status === 'rejected'
        ? receipt.outcome.reason
        : 'The computer could not confirm this linked draft.';
      setReview({ status: 'failed', message });
      return;
    }
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      const result = await read(active.hostId, active.id, controller.signal);
      if (controller.signal.aborted) return;
      if (!result.ok) {
        setReview({ status: 'failed', message: result.error.message });
        return;
      }
      setReview(result.value.value);
      if (result.value.value.status === 'preparing') timer = setTimeout(() => void poll(), 1000);
    };
    void poll();
    return () => {
      controller.abort();
      clearTimeout(timer);
    };
  }, [active, online, foreground, pending, read, receipt?.id, receipt?.outcome.status]);

  function prepare() {
    if (!link || !hostId || !canPrepare) return;
    const target = link.pullRequest
      ? { pull_request: pullRequestUrl(link.pullRequest) }
      : { repository: link.repository, provider: link.provider, model: link.model };
    void model.openLinkedDraft(hostId, target, (id) => {
      setReview({ status: 'preparing' });
      setRequest({ hostId, id, commandId: id, link });
    });
  }

  function confirm(choice: LinkedDraftChoice) {
    if (!active || !canPrepare) return;
    void model.continueLinkedDraft(active.hostId, active.id, choice, (commandId) => {
      setReview({ status: 'preparing' });
      setRequest({ ...active, commandId });
    });
  }

  return { supported, canPrepare, status, prepare, confirm };
}
