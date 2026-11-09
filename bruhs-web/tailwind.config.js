const plugin = require("tailwindcss/plugin");

// Self-contained flatten ({longan:{500:"#…"}} → {"longan-500":"#…"}) —
// Trunk's standalone tailwindcss binary can't require() tailwind's internal
// flattenColorPalette helper.
function flattenColors(obj, prefix = "") {
  const out = {};
  for (const [key, value] of Object.entries(obj)) {
    if (typeof value === "string") {
      out[key === "DEFAULT" ? prefix.replace(/-$/, "") : prefix + key] = value;
    } else if (value && typeof value === "object") {
      Object.assign(out, flattenColors(value, `${prefix}${key}-`));
    }
  }
  return out;
}

module.exports = {
  content: ["./index.html", "./src/**/*.rs"],
  theme: {
    extend: {
      // Trunk bundles Tailwind 3.3, before spacing-based min-size utilities.
      // 10 is the 40px control. 11 matches the 44px mobile menu target.
      minHeight: { 10: "40px", 11: "44px" },
      minWidth: { 10: "40px", 11: "44px" },
      colors: {
        longan: {
          50: "#f6f3ee",
          100: "#e8e1d4",
          200: "#cfc0a9",
          300: "#b39c7d",
          400: "#917a5b",
          500: "#6f5d44",
          600: "#594a36",
          700: "#443a2c",
          800: "#2f2922",
          900: "#1d1916",
          950: "#141210",
        },
        // sRGB of the oklch stops in vendor/bruhs.css. 400 is the darkest
        // informative text that stays above 4.5:1 on longan-950, 900, and
        // 800. 500 is placeholder and subtle text. 600 and darker are
        // separators and tracks: 600 clears the canvas only.
        rambutan: {
          50: "#f8f5f1",
          100: "#f5ede7",
          200: "#ede1d8",
          300: "#dfcfc3",
          400: "#c3afa1",
          500: "#ab998f",
          600: "#8b7c74",
          700: "#716560",
          800: "#5d524f",
          900: "#4d4542",
          950: "#1f1b1a",
        },
        persimmon: {
          50: "#fef3ec",
          100: "#fbe1cc",
          200: "#f7c194",
          300: "#f29e58",
          400: "#ec7f2c",
          500: "#df6815",
          600: "#bd5310",
          700: "#993f0e",
          800: "#7a330e",
          900: "#5e290c",
          950: "#391704",
        },
        pandan: {
          50: "#eef9ec",
          100: "#d4f0cd",
          200: "#abe19f",
          300: "#7fcc6e",
          400: "#5ab44a",
          500: "#3f9931",
          600: "#317926",
          700: "#285e21",
          800: "#234c1e",
          900: "#1d3f1a",
          950: "#0c2009",
        },
        lychee: {
          50: "#fef2f2",
          100: "#fde2e1",
          200: "#fcc6c4",
          300: "#f8a09c",
          400: "#f06b65",
          500: "#df3f37",
          600: "#bf2a23",
          700: "#9a221c",
          800: "#7c1f1a",
          900: "#681f1b",
          950: "#380a08",
        },
        durian: {
          50: "#fef9ec",
          100: "#fbf0c8",
          200: "#f6dd8c",
          300: "#f0c450",
          400: "#e9aa1e",
          500: "#cd8e0d",
          600: "#a26b09",
          700: "#824f0c",
          800: "#6a3f10",
          900: "#5a3411",
          950: "#321b06",
        },
        blueberry: {
          400: "#5b9adf",
          500: "#3a7fcb",
        },
        mangosteen: {
          400: "#a87fcc",
          500: "#8a5fb5",
        },
        dragonfruit: {
          400: "#e864a3",
          500: "#d04388",
        },
      },
      fontFamily: {
        neon: ['"Monaspace Neon"', "ui-monospace", "monospace"],
        argon: ['"Monaspace Argon"', "ui-monospace", "monospace"],
        krypton: ['"Monaspace Krypton"', "ui-monospace", "monospace"],
        radon: ['"Monaspace Radon"', "ui-monospace", "monospace"],
        xenon: ['"Monaspace Xenon"', "ui-monospace", "monospace"],
      },
      borderRadius: {
        seed: "4px",
        grape: "8px",
        lychee: "14px",
        mango: "22px",
        pomelo: "36px",
        orb: "9999px",
      },
    },
  },
  plugins: [
    // bruhs elevation system: 1px inset ring instead of drop shadows.
    // Generates `inset-ring` (base) and `inset-ring-{color}[/{alpha}]` for
    // the local palette. The @bruhs/theme CDN stylesheet already ships these
    // utilities; this mirrors them in the local build (same pattern as the
    // color/radius tokens above) so the app keeps its elevation when the CDN
    // is unreachable, e.g. offline dev.
    plugin(function ({ addUtilities, matchUtilities, theme }) {
      addUtilities({
        ".inset-ring": {
          "box-shadow":
            "inset 0 0 0 1px var(--tw-inset-ring-color, transparent)",
        },
      });
      matchUtilities(
        {
          "inset-ring": (value) => ({
            "--tw-inset-ring-color": value,
            "box-shadow": "inset 0 0 0 1px var(--tw-inset-ring-color)",
          }),
        },
        { values: flattenColors(theme("colors")), type: "color" }
      );
      // `size-*` (width + height together) shipped in Tailwind 3.4; Trunk's
      // bundled binary is 3.3, so mirror it here.
      matchUtilities(
        { size: (value) => ({ width: value, height: value }) },
        { values: { ...theme("spacing"), full: "100%" } }
      );
    }),
  ],
};
