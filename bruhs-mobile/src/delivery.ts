import {
  type Host,
  type NativeCommand,
  type Receipt,
  type Result,
  type Session,
  failure,
  success,
} from './domain.ts';

export type Recovery = Readonly<
  | { kind: 'clear' }
  | { kind: 'receipt'; receipt: Receipt }
  | { kind: 'retry'; command: NativeCommand }
  | { kind: 'review'; message: string }
>;

export function recoverDelivery(pending: NativeCommand | null, session: Session): Recovery {
  if (!pending)
    return session.last_command
      ? { kind: 'receipt', receipt: session.last_command }
      : { kind: 'clear' };
  if (session.last_command?.id === pending.id && session.last_command.sequence === pending.sequence)
    return { kind: 'receipt', receipt: session.last_command };
  if (session.next_sequence === pending.sequence) return { kind: 'retry', command: pending };
  return {
    kind: 'review',
    message:
      'This command’s delivery cannot be established. Check the computer before sending another command.',
  };
}

export function deliveryStatus(receipt: Receipt): string {
  switch (receipt.outcome.status) {
    case 'pending':
      return 'Reserved on the computer. Check this same command to avoid sending it twice.';
    case 'accepted':
      return receipt.outcome.queued
        ? 'Added to the computer’s queue.'
        : 'Accepted by the computer.';
    case 'rejected':
      return receipt.outcome.reason;
    case 'unconfirmed':
      return 'Delivery is unconfirmed. This command will not be replayed. Check the computer.';
  }
}

export function canMutate(
  host: Host,
  online: boolean,
  pending: NativeCommand | null,
  session: Session | null,
  scope: 'prompt' | 'approve',
): boolean {
  return (
    online &&
    !pending &&
    host.authority === 'desktop' &&
    !!session &&
    !session.device.revoked &&
    session.device.expires_at_ms > Date.now() &&
    session.device.scopes.includes(scope)
  );
}

export function sameScope(first: Host, second: Host): boolean {
  return (
    first.id === second.id &&
    first.origin === second.origin &&
    first.authority === second.authority &&
    first.token === second.token
  );
}
export function checkSelection(selected: Host | null, operationHost: Host): Result<void> {
  return selected && sameScope(selected, operationHost)
    ? success(undefined)
    : failure('conflict', 'The selected host changed. Read its current state before continuing.');
}
