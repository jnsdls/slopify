// Injected at document start into every frame, before the SDK loads. The SDK does its playback inside a
// cross-origin iframe (sdk.scdn.co/embedded), so this has to run in subframes too.
(() => {
  if (window.__spikeInjected) return;
  window.__spikeInjected = true;
  const FRAME = window.top === window ? 'main' : 'sub:' + location.origin + location.pathname;
  const post = (o) => {
    try {
      window.webkit.messageHandlers.ipc.postMessage(JSON.stringify({ ...o, frame: FRAME }));
    } catch (_) {}
  };
  const redact = (u) =>
    String(u)
      .replace(/((?:access_)?token|code|refresh_token)=[^&#]+/gi, '$1=REDACTED')
      .replace(/Bearer\s+[A-Za-z0-9._\-]+/g, 'Bearer REDACTED');
  const fmt = (x) => {
    if (typeof x === 'string') return x;
    if (x instanceof Error) return `${x.name}: ${x.message}`;
    try {
      return JSON.stringify(x);
    } catch (_) {
      return String(x);
    }
  };
  window.__spikePost = post;

  for (const level of ['log', 'info', 'warn', 'error', 'debug']) {
    const orig = console[level].bind(console);
    console[level] = (...a) => {
      post({ t: 'console', level, msg: redact(a.map(fmt).join(' ')) });
      orig(...a);
    };
  }
  window.addEventListener('error', (e) => post({ t: 'console', level: 'onerror', msg: `${e.message} ${e.filename}:${e.lineno}` }));
  window.addEventListener('unhandledrejection', (e) => post({ t: 'console', level: 'unhandledrejection', msg: redact(fmt(e.reason)) }));

  // Network: URL, method, status. Never headers or bodies.
  const origFetch = window.fetch;
  window.fetch = async function (input, init) {
    const url = redact(typeof input === 'string' ? input : input && input.url);
    const method = (init && init.method) || (input && input.method) || 'GET';
    try {
      const r = await origFetch.apply(this, arguments);
      post({ t: 'net', kind: 'fetch', method, url, status: r.status });
      return r;
    } catch (e) {
      post({ t: 'net', kind: 'fetch', method, url, error: fmt(e) });
      throw e;
    }
  };
  const xOpen = XMLHttpRequest.prototype.open;
  const xSend = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.open = function (m, u) {
    this.__spike = { m, u: redact(u) };
    return xOpen.apply(this, arguments);
  };
  XMLHttpRequest.prototype.send = function () {
    this.addEventListener('loadend', () =>
      post({ t: 'net', kind: 'xhr', method: this.__spike && this.__spike.m, url: this.__spike && this.__spike.u, status: this.status }),
    );
    return xSend.apply(this, arguments);
  };

  // EME: which key system is asked for and which one is granted.
  if (navigator.requestMediaKeySystemAccess) {
    const orig = navigator.requestMediaKeySystemAccess.bind(navigator);
    navigator.requestMediaKeySystemAccess = async (keySystem, configs) => {
      post({ t: 'eme', op: 'request', keySystem, configs: fmt(configs).slice(0, 800) });
      try {
        const access = await orig(keySystem, configs);
        post({ t: 'eme', op: 'granted', keySystem: access.keySystem, config: fmt(access.getConfiguration()).slice(0, 800) });
        return access;
      } catch (e) {
        post({ t: 'eme', op: 'denied', keySystem, error: fmt(e) });
        throw e;
      }
    };
  }
  if (window.MediaKeys) {
    const create = MediaKeys.prototype.createSession;
    MediaKeys.prototype.createSession = function (type) {
      const s = create.apply(this, arguments);
      post({ t: 'eme', op: 'createSession', type: type || 'temporary' });
      s.addEventListener('message', (e) => post({ t: 'eme', op: 'session-message', messageType: e.messageType, bytes: e.message.byteLength }));
      s.addEventListener('keystatuseschange', () => {
        const st = [];
        s.keyStatuses.forEach((v) => st.push(v));
        post({ t: 'eme', op: 'keystatuses', statuses: st.join(',') });
      });
      const gen = s.generateRequest;
      s.generateRequest = function (initDataType, initData) {
        post({ t: 'eme', op: 'generateRequest', initDataType, bytes: initData && initData.byteLength });
        return gen.apply(this, arguments).catch((e) => {
          post({ t: 'eme', op: 'generateRequest-failed', error: fmt(e) });
          throw e;
        });
      };
      const upd = s.update;
      s.update = function (resp) {
        post({ t: 'eme', op: 'update', bytes: resp && resp.byteLength });
        return upd.apply(this, arguments).then(
          (v) => (post({ t: 'eme', op: 'update-ok' }), v),
          (e) => {
            post({ t: 'eme', op: 'update-failed', error: fmt(e) });
            throw e;
          },
        );
      };
      return s;
    };
    const setServerCert = MediaKeys.prototype.setServerCertificate;
    MediaKeys.prototype.setServerCertificate = function (cert) {
      post({ t: 'eme', op: 'setServerCertificate', bytes: cert && cert.byteLength });
      return setServerCert.apply(this, arguments);
    };
  }
  if (window.WebKitMediaKeys) {
    const Orig = window.WebKitMediaKeys;
    window.WebKitMediaKeys = function (ks) {
      post({ t: 'eme', op: 'legacy WebKitMediaKeys', keySystem: ks });
      return new Orig(ks);
    };
    window.WebKitMediaKeys.isTypeSupported = Orig.isTypeSupported.bind(Orig);
    window.WebKitMediaKeys.prototype = Orig.prototype;
  }

  // Media elements: play() result tells us whether autoplay was blocked.
  const play = HTMLMediaElement.prototype.play;
  HTMLMediaElement.prototype.play = function () {
    const p = play.apply(this, arguments);
    post({ t: 'media', op: 'play()', tag: this.tagName });
    if (p && p.then) p.then(() => post({ t: 'media', op: 'play() resolved' }), (e) => post({ t: 'media', op: 'play() rejected', error: fmt(e) }));
    return p;
  };
  const setMediaKeys = HTMLMediaElement.prototype.setMediaKeys;
  if (setMediaKeys) {
    HTMLMediaElement.prototype.setMediaKeys = function (mk) {
      post({ t: 'eme', op: 'setMediaKeys', set: !!mk });
      return setMediaKeys.apply(this, arguments);
    };
  }

  const env = () =>
    post({
      t: 'env',
      href: redact(location.href),
      origin: location.origin,
      isSecureContext: window.isSecureContext,
      ua: navigator.userAgent,
      eme: !!navigator.requestMediaKeySystemAccess,
      legacyFps: !!window.WebKitMediaKeys,
      mse: !!window.MediaSource,
      managedMse: !!window.ManagedMediaSource,
      mediaSession: !!navigator.mediaSession,
    });
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', env);
  else env();
})();
