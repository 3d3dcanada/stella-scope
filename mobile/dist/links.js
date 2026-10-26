// The addresses the Stella app is opened with. Pure, so it is tested under node (scripts/mobile-links-tests.mjs).
//   stella://pair/<link id>?code=123456   add the computer and pair this phone
//   stella://link/<link id>               add the computer
//   stella://p/<printer>[?link=<link id>] open that printer (an alert was tapped)
(function (root) {
  const HEX = /^[0-9a-f]{64}$/i;
  function parse(text) {
    const t = String(text || '').trim();
    let m = /^stella:\/\/pair\/([0-9a-f]{64})\?code=(\d{6})/i.exec(t);
    if (m) return { kind: 'pair', link: m[1].toLowerCase(), code: m[2] };
    m = /^stella:\/\/link\/([0-9a-f]{64})\/?$/i.exec(t);
    if (m) return { kind: 'link', link: m[1].toLowerCase() };
    m = /^stella:\/\/p\/([a-z0-9-]{1,32})\/?(?:\?(.*))?$/i.exec(t);
    if (m) {
      const link = (new URLSearchParams(m[2] || '').get('link') || '').toLowerCase();
      return { kind: 'printer', printer: m[1].toLowerCase(), link: HEX.test(link) ? link : null };
    }
    return { kind: 'unknown' };
  }
  // Which saved computer a printer address opens: the one it names, else the only one, else the last used.
  // Never a computer this phone has not been paired with.
  function pick(list, link, last) {
    if (link) return list.find((p) => p.id === link) || null;
    if (list.length === 1) return list[0];
    return list.find((p) => p.id === last) || null;
  }
  const api = { parse, pick };
  if (typeof module !== 'undefined') module.exports = api;
  else root.StellaLinks = api;
})(typeof window !== 'undefined' ? window : globalThis);
