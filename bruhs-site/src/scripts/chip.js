// Which chip a Mac visitor has, so an Intel Mac isn't offered the Apple
// silicon download. Every Mac browser's user agent says "Intel Mac OS X", so
// this reads other signals:
//
// - Chrome, Edge, Arc, Brave: the `architecture` client hint, "arm" or "x86".
// - Chrome and Firefox: the WebGL renderer names the GPU ("Apple M2 Max",
//   "Intel Iris", "AMD Radeon").
// - Safari: the renderer is just "Apple GPU" on every Mac, but only Apple
//   silicon GPUs support ASTC textures.
//
// Anything it can't place is null, and the page keeps the Mac download.

/**
 * @typedef {object} ChipSignals
 * @property {string | null} [architecture] userAgentData architecture hint
 * @property {string | null} [renderer] WebGL renderer (unmasked when the browser allows)
 * @property {boolean | null} [astc] WEBGL_compressed_texture_astc is supported
 */

/**
 * @param {ChipSignals} signals
 * @returns {"apple" | "intel" | null}
 */
export function chipFrom({ architecture = null, renderer = null, astc = null }) {
  // A named GPU settles it, even over the architecture hint: Chrome running
  // under Rosetta reports x86 on an Apple silicon Mac.
  if (renderer) {
    if (/\bApple M\d/.test(renderer)) return "apple";
    if (/\b(Intel|AMD|Radeon|NVIDIA|GeForce)\b/i.test(renderer)) return "intel";
  }
  if (architecture === "arm") return "apple";
  if (architecture === "x86") return "intel";
  // Safari masks the name; only its generic "Apple GPU" makes ASTC a tell.
  if (renderer && /^Apple GPU$/.test(renderer.trim()) && astc !== null) return astc ? "apple" : "intel";
  return null;
}

/**
 * Reads the signals in this browser. WebGL is only touched when the hint
 * can't answer alone, and the context is released right away.
 * @returns {Promise<"apple" | "intel" | null>}
 */
export async function detectChip() {
  /** @type {ChipSignals} */
  const signals = {};
  const hints = /** @type {{ getHighEntropyValues?: (keys: string[]) => Promise<{ architecture?: string }> } | undefined} */ (
    /** @type {any} */ (navigator).userAgentData
  );
  try {
    signals.architecture = (await hints?.getHighEntropyValues?.(["architecture"]))?.architecture ?? null;
  } catch {
    signals.architecture = null;
  }
  // "arm" can't be Rosetta; anything else gets a second opinion from the GPU.
  if (signals.architecture === "arm") return "apple";
  try {
    const gl = document.createElement("canvas").getContext("webgl");
    if (gl) {
      const debug = gl.getExtension("WEBGL_debug_renderer_info");
      signals.renderer = String(gl.getParameter(debug ? debug.UNMASKED_RENDERER_WEBGL : gl.RENDERER) ?? "");
      signals.astc = gl.getExtension("WEBGL_compressed_texture_astc") !== null;
      gl.getExtension("WEBGL_lose_context")?.loseContext();
    }
  } catch {
    // No WebGL: leave it to the hint, or to nothing.
  }
  return chipFrom(signals);
}
