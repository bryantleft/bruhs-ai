import { fetch } from 'expo/fetch';
import type { Transport } from './client';

export const transport: Transport = async (url, init) => {
  // Expo's native transport implements redirect:error on both NSURLSession and
  // OkHttp. React Native's legacy global fetch does not honor that contract.
  const response = await fetch(url, {
    method: init.method,
    body: init.body,
    signal: init.signal,
    credentials: 'omit',
    redirect: 'error',
    headers: {
      'Cache-Control': 'no-store',
      ...(init.token ? { Authorization: `Bearer ${init.token}` } : {}),
      ...(init.body !== undefined ? { 'Content-Type': 'application/json' } : {}),
    },
  });
  if (response.redirected)
    throw new Error('The host redirected this request. Use its final trusted HTTPS address.');
  const reader = response.body?.getReader();
  if (!reader) throw new Error('The host returned no response body.');
  const decoder = new TextDecoder();
  let text = '',
    count = 0;
  try {
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      count += chunk.value.byteLength;
      if (count > init.maxBytes)
        throw new Error(
          'The host response exceeded the mobile page budget. Review large results on your computer.',
        );
      text += decoder.decode(chunk.value, { stream: true });
    }
    text += decoder.decode();
  } finally {
    await reader.cancel();
  }
  return { status: response.status, text };
};
