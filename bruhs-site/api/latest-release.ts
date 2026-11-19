// Cache the public release metadata on bruhs.ai so visitors share GitHub's rate limit.
export default {
  async fetch(): Promise<Response> {
    const upstream = await fetch("https://api.github.com/repos/bryantleft/bruhs-ai/releases/latest", {
      headers: { Accept: "application/vnd.github+json", "User-Agent": "bruhs-downloads" },
    });
    if (!upstream.ok) return new Response(null, { status: 502 });
    return new Response(upstream.body, {
      headers: {
        "Content-Type": "application/json",
        "Cache-Control": upstream.headers.get("cache-control") ?? "no-cache",
      },
    });
  },
};
