import * as Crypto from 'expo-crypto';
import { Vault } from './vault';

// Web is a preview of native UI. Credentials never fall back to localStorage.
const memory = new Map<string, string>();
export const vault = new Vault(
  {
    get: async (key) => memory.get(key) ?? null,
    set: async (key, value) => {
      memory.set(key, value);
    },
    remove: async (key) => {
      memory.delete(key);
    },
  },
  Crypto.randomUUID,
  __DEV__,
);
export const ephemeralStorage = true;
