// This is a codec boundary, not a clone operation. Reject representations that
// JSON would silently change; the caller retains the structured legacy decoder.
export function stringifyMetadata(value) {
  const pending = [value];
  const seen = new Set();
  while (pending.length) {
    const item = pending.pop();
    if (item === null || typeof item === 'string' || typeof item === 'boolean') continue;
    if (typeof item === 'number') {
      if (!Number.isFinite(item) || Object.is(item, -0)
          || (Number.isInteger(item) && !Number.isSafeInteger(item))) return undefined;
      continue;
    }
    if (typeof item !== 'object' || seen.has(item)) return undefined;
    const prototype = Object.getPrototypeOf(item);
    if (!Array.isArray(item) && prototype !== Object.prototype && prototype !== null) return undefined;
    seen.add(item);
    for (const child of Array.isArray(item) ? item : Object.values(item)) pending.push(child);
  }
  return JSON.stringify(value);
}
