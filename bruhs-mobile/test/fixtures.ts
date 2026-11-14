import type { Host, NativeCommand, Receipt, Session } from '../src/domain.ts';

export const uuid = 'c6c95269-6909-4c46-ab44-0873bd173a6d';
export const otherUuid = '32493054-d400-465c-bd1e-d09a3d49c12a';
export const desktop: Host = {
  id: uuid,
  name: 'Laptop',
  origin: 'https://laptop.example:9418',
  authority: 'desktop',
  token: 'desktop-token',
};
export const tasks: Host = {
  id: otherUuid,
  name: 'Task host',
  origin: 'https://tasks.example:9417',
  authority: 'tasks',
  token: 'task-owner-token',
};
export const command = {
  id: otherUuid,
  sequence: 7,
  action: { kind: 'prompt', thread_id: uuid, text: 'Continue the implementation.' },
} satisfies NativeCommand;
export const accepted: Receipt = {
  id: command.id,
  sequence: command.sequence,
  outcome: { status: 'accepted', queued: false },
};
export const session: Session = {
  device: {
    id: uuid,
    name: 'Test phone',
    scopes: ['read', 'prompt', 'approve', 'diff'],
    expires_at_ms: Date.now() + 3600_000,
    last_seen_ms: null,
    revoked: false,
  },
  next_sequence: 7,
  last_command: null,
};
