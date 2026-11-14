import * as SecureStore from 'expo-secure-store';
import * as Crypto from 'expo-crypto';
import { Vault } from './vault';

const options: SecureStore.SecureStoreOptions = {
  keychainAccessible: SecureStore.WHEN_UNLOCKED_THIS_DEVICE_ONLY,
  keychainService: 'dev.bruhs.mobile',
};
export const vault = new Vault(
  {
    get: (key) => SecureStore.getItemAsync(key, options),
    set: (key, value) => SecureStore.setItemAsync(key, value, options),
    remove: (key) => SecureStore.deleteItemAsync(key, options),
  },
  Crypto.randomUUID,
  __DEV__,
);
export const ephemeralStorage = false;
