// Marks the device on <html data-os> before the first paint, so the page
// offers what this device can use: the Mac download on a Mac, "Send to your
// Mac" on phones and tablets, and run-from-source on Windows and Linux
// (site.css, .for-mac / .for-phone / .for-pc). It's a plain script in <head>
// rather than part of the module bundle so the wrong button never flashes.
// Without it, everyone gets the Mac download.
(function () {
  var nav = navigator;
  var ua = nav.userAgent || "";
  var hints = /** @type {{ platform?: string, mobile?: boolean } | undefined} */ (/** @type {any} */ (nav).userAgentData);
  var platform = (hints && hints.platform) || nav.platform || "";
  var os = "mac";
  // iPadOS asks for desktop sites with a Mac user agent; only touch gives it away.
  var touchMac = /Mac/.test(platform + ua) && nav.maxTouchPoints > 1;
  if ((hints && hints.mobile) || touchMac || /iPhone|iPad|iPod|Android|Mobile/.test(ua)) os = "phone";
  else if (/Win|Linux|X11|CrOS/.test(platform) || /Windows|Linux|X11|CrOS/.test(ua)) os = "pc";
  document.documentElement.setAttribute("data-os", os);
})();
