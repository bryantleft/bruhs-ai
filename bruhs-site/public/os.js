// Classify desktop platforms without mistaking an iPad in desktop mode for a Mac.
(function () {
  var ua = navigator.userAgent || "";
  var hints = /** @type {any} */ (navigator).userAgentData;
  var platform = (hints && hints.platform) || navigator.platform || "";
  var device = platform + " " + ua;
  var os = "other";
  if ((hints && hints.mobile) || (/Mac/.test(device) && navigator.maxTouchPoints > 1) || /iPhone|iPad|iPod|Android|Mobile/.test(ua)) os = "phone";
  else if (/Win/.test(device)) os = "windows";
  else if (/Mac/.test(device)) os = "mac";
  else if (/Linux|X11/.test(device) && !/CrOS|Chrome OS|arm|aarch/i.test(device)) os = "linux";
  document.documentElement.setAttribute("data-os", os);
})();
