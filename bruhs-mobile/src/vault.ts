import { z } from 'zod';
import {
  type Host,
  type NativeCommand,
  type Result,
  commandSchema,
  failure,
  hostSchema,
  messageOf,
  parse,
  success,
  validateHost,
} from './domain.ts';

export interface SecretDriver {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  remove(key: string): Promise<void>;
}
export const uncertainTaskSchema = z.object({
  id: z.string().uuid(),
  description: z.string(),
  at: z.number().int().nonnegative(),
});
export type UncertainTask = z.infer<typeof uncertainTaskSchema>;
const pointerSchema = z.object({
  generation: z.string().uuid(),
  count: z.number().int().positive().max(256),
});
// The native broker accepts a 64KiB frame. A saved command uses the same bound.
const RECORD_BYTES = 64 * 1024;
// SecureStore documents iOS failures above 2KiB per value. 256 Unicode scalars
// occupy at most 1KiB UTF-8, leaving half that contract for platform variation.
const SCALARS_PER_SHARD = 256;

export class Vault {
  private writers = new Map<string, Promise<unknown>>();
  private readonly driver: SecretDriver;
  private readonly uuid: () => string;
  private readonly development: boolean;
  constructor(driver: SecretDriver, uuid: () => string, development: boolean) {
    this.driver = driver;
    this.uuid = uuid;
    this.development = development;
  }

  private async read<T>(key: string, schema: z.ZodType<T>): Promise<Result<T | null>> {
    try {
      const raw = await this.driver.get(key);
      if (raw === null) return success(null);
      const pointer = parse(pointerSchema, JSON.parse(raw));
      if (!pointer.ok)
        return failure(
          'storage',
          'Saved credentials are unreadable. Remove this host and pair again.',
        );
      const shards = await Promise.all(
        Array.from({ length: pointer.value.count }, (_, index) =>
          this.driver.get(`${key}.${pointer.value.generation}.${index}`),
        ),
      );
      if (shards.some((value) => value === null))
        return failure(
          'storage',
          'Saved credentials are incomplete. Remove this host and pair again.',
        );
      return parse(schema, JSON.parse(shards.join('')));
    } catch (error) {
      return failure('storage', `Could not read secure device storage. ${messageOf(error)}`);
    }
  }

  private async write(key: string, value: unknown | null): Promise<Result<void>> {
    const previous = this.writers.get(key) ?? Promise.resolve();
    const work = previous.then(async (): Promise<Result<void>> => {
      let newKeys: string[] = [];
      let committed = false;
      try {
        const old = await this.driver.get(key);
        let oldPointer: ReturnType<typeof pointerSchema.safeParse> | null = null;
        try {
          oldPointer = old === null ? null : pointerSchema.safeParse(JSON.parse(old));
        } catch {
          /* A damaged pointer must still be removable. */
        }
        if (value === null) await this.driver.remove(key);
        else {
          const json = JSON.stringify(value);
          if (new TextEncoder().encode(json).byteLength > RECORD_BYTES)
            return failure(
              'storage',
              'This saved record exceeds the mobile command budget. Shorten the draft or remove unused hosts.',
            );
          const scalars = Array.from(json),
            generation = this.uuid(),
            count = Math.ceil(scalars.length / SCALARS_PER_SHARD);
          newKeys = Array.from({ length: count }, (_, index) => `${key}.${generation}.${index}`);
          // At most 256 independent writes, bounded by the 64KiB record contract.
          const writes = await Promise.allSettled(
            newKeys.map((part, index) =>
              this.driver.set(
                part,
                scalars.slice(index * SCALARS_PER_SHARD, (index + 1) * SCALARS_PER_SHARD).join(''),
              ),
            ),
          );
          const failed = writes.find((result) => result.status === 'rejected');
          if (failed?.status === 'rejected') throw failed.reason;
          await this.driver.set(key, JSON.stringify({ generation, count }));
        }
        committed = true;
        if (oldPointer?.success)
          await Promise.all(
            Array.from({ length: oldPointer.data.count }, (_, index) =>
              this.driver.remove(`${key}.${oldPointer.data.generation}.${index}`),
            ),
          );
        return success(undefined);
      } catch (error) {
        if (!committed)
          await Promise.all(newKeys.map((part) => this.driver.remove(part).catch(() => undefined)));
        return failure(
          'storage',
          `Secure storage could not complete this operation. ${messageOf(error)}`,
        );
      }
    });
    this.writers.set(key, work);
    const result = await work;
    if (this.writers.get(key) === work) this.writers.delete(key);
    return result;
  }

  async hosts(): Promise<Result<readonly Host[]>> {
    const ids = await this.read('bruhs.catalog', z.array(z.string().uuid()));
    if (!ids.ok) return ids;
    const hosts = await Promise.all(
      (ids.value ?? []).map((id) => this.read(`bruhs.host.${id}`, hostSchema)),
    );
    const values: Host[] = [];
    for (const host of hosts) {
      if (!host.ok) return host;
      if (!host.value)
        return failure(
          'storage',
          'A saved host is incomplete. Remove its saved app data and pair again.',
        );
      const valid = validateHost(host.value, this.development);
      if (!valid.ok) return valid;
      values.push(valid.value);
    }
    return success(values);
  }

  async saveHost(host: Host): Promise<Result<void>> {
    const valid = validateHost(host, this.development);
    if (!valid.ok) return valid;
    const ids = await this.read('bruhs.catalog', z.array(z.string().uuid()));
    if (!ids.ok) return ids;
    const saved = await this.write(`bruhs.host.${host.id}`, host);
    return saved.ok
      ? this.write('bruhs.catalog', [...new Set([...(ids.value ?? []), host.id])])
      : saved;
  }

  async removeHost(id: string): Promise<Result<void>> {
    const ids = await this.read('bruhs.catalog', z.array(z.string().uuid()));
    if (!ids.ok) return ids;
    const removed = await this.write(
      'bruhs.catalog',
      (ids.value ?? []).filter((value) => value !== id),
    );
    if (!removed.ok) return removed;
    const values = await Promise.all([
      this.write(`bruhs.host.${id}`, null),
      this.write(`bruhs.pending.${id}`, null),
      this.write(`bruhs.task-pending.${id}`, null),
      this.write(`bruhs.drafts.${id}`, null),
    ]);
    return values.find((result) => !result.ok) ?? success(undefined);
  }

  pending(host: Host): Promise<Result<NativeCommand | null>> {
    return host.authority === 'desktop'
      ? this.read(`bruhs.pending.${host.id}`, commandSchema)
      : Promise.resolve(failure('invalid', 'Native commands belong to a desktop pairing.'));
  }
  savePending(host: Host, command: NativeCommand | null): Promise<Result<void>> {
    return host.authority === 'desktop'
      ? this.write(`bruhs.pending.${host.id}`, command)
      : Promise.resolve(failure('invalid', 'Native commands belong to a desktop pairing.'));
  }
  taskPending(host: Host): Promise<Result<UncertainTask | null>> {
    return host.authority === 'tasks'
      ? this.read(`bruhs.task-pending.${host.id}`, uncertainTaskSchema)
      : Promise.resolve(failure('invalid', 'Task delivery belongs to a durable task connection.'));
  }
  saveTaskPending(host: Host, pending: UncertainTask | null): Promise<Result<void>> {
    return host.authority === 'tasks'
      ? this.write(`bruhs.task-pending.${host.id}`, pending)
      : Promise.resolve(failure('invalid', 'Task delivery belongs to a durable task connection.'));
  }
  drafts(host: Host): Promise<Result<Record<string, string> | null>> {
    return this.read(`bruhs.drafts.${host.id}`, z.record(z.string(), z.string()));
  }
  saveDrafts(host: Host, drafts: Readonly<Record<string, string>>): Promise<Result<void>> {
    return this.write(`bruhs.drafts.${host.id}`, drafts);
  }
}
