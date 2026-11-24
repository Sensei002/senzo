// SENZO Nuvio provider compatibility layer: host globals, timers, fetch.
//
// Provider scripts were written for the Nuvio QuickJS host. They expect a
// small set of web and Node APIs that QuickJS does not provide. These shims
// are deliberately narrow: no DOM, no Node fs/child_process, no eval of
// downloaded code beyond the provider script itself. Network access goes
// through __senzo_fetch, which enforces public-host checks in Rust.

(function () {
  'use strict';

  if (typeof globalThis.global === 'undefined') globalThis.global = globalThis;
  if (typeof globalThis.window === 'undefined') globalThis.window = globalThis;
  if (typeof globalThis.self === 'undefined') globalThis.self = globalThis;

  globalThis.SCRAPER_ID = __senzo_scraper_id();
  globalThis.SCRAPER_SETTINGS = JSON.parse(__senzo_settings_json() || '{}');
  if (typeof globalThis.TMDB_API_KEY === 'undefined') {
    // This is a routing marker, not a usable TMDB credential. Rust rewrites
    // api.themoviedb.org requests to the SENZO metadata gateway and removes
    // this parameter before the request leaves the machine.
    globalThis.TMDB_API_KEY = 'senzo-gateway';
  }

  var __plugin_module = { exports: {} };
  if (typeof globalThis.module === 'undefined') globalThis.module = __plugin_module;
  if (typeof globalThis.exports === 'undefined') globalThis.exports = globalThis.module.exports;
  if (typeof globalThis.process === 'undefined') {
    globalThis.process = {
      env: { NODE_ENV: 'production' },
      argv: [],
      platform: 'senzo',
      version: 'v20.0.0',
      versions: {},
      nextTick: function (callback) { Promise.resolve().then(callback); },
      on: function () {},
      once: function () {},
      off: function () {},
      emit: function () { return false; },
      cwd: function () { return '/'; }
    };
  }

  if (typeof globalThis.console === 'undefined') globalThis.console = {};
  ['log', 'info', 'warn', 'error', 'debug'].forEach(function (level) {
    globalThis.console[level] = function () {
      var parts = [];
      for (var i = 0; i < arguments.length; i++) {
        var value = arguments[i];
        if (value instanceof Error) parts.push(value.stack || value.message || String(value));
        else if (typeof value === 'object') {
          try { parts.push(JSON.stringify(value)); } catch (error) { parts.push(String(value)); }
        } else parts.push(String(value));
      }
      try { __senzo_log(level, parts.join(' ')); } catch (error) {}
    };
  });

  // QuickJS has no event loop. The host sleeps on the provider's worker thread
  // and then resolves the callback through the microtask queue. This keeps
  // `await new Promise(resolve => setTimeout(resolve, 100))` working without a
  // second thread or a runaway timer queue.
  var __plugin_timer_id = 0;
  var __plugin_timers = Object.create(null);

  function __plugin_delay(value, interval) {
    var duration = Number(value);
    if (!isFinite(duration) || duration < 0) duration = 0;
    duration = Math.min(Math.floor(duration), 60000);
    return interval && duration < 1 ? 1 : duration;
  }

  globalThis.setTimeout = function (callback, delay) {
    if (typeof callback !== 'function') throw new TypeError('Timer callback must be a function');
    var id = ++__plugin_timer_id;
    var args = Array.prototype.slice.call(arguments, 2);
    __plugin_timers[id] = true;
    __senzo_sleep(__plugin_delay(delay, false));
    Promise.resolve().then(function () {
      if (!__plugin_timers[id]) return;
      delete __plugin_timers[id];
      try { callback.apply(globalThis, args); } catch (error) { console.error('[PluginRuntime] timer callback failed:', error); }
    });
    return id;
  };
  globalThis.clearTimeout = function (id) { delete __plugin_timers[id]; };
  globalThis.setInterval = function (callback, delay) {
    if (typeof callback !== 'function') throw new TypeError('Timer callback must be a function');
    // A synchronous host cannot run a true interval without blocking forever.
    // Run one tick so polling loops make progress, then let the provider finish.
    return globalThis.setTimeout(callback, delay, ...Array.prototype.slice.call(arguments, 2));
  };
  globalThis.clearInterval = globalThis.clearTimeout;
  globalThis.queueMicrotask = function (callback) { Promise.resolve().then(callback); };

  var AbortSignal = function () {
    this.aborted = false;
    this.reason = undefined;
    this._listeners = [];
  };
  AbortSignal.prototype.addEventListener = function (type, listener) {
    if (type === 'abort' && typeof listener === 'function') this._listeners.push(listener);
  };
  AbortSignal.prototype.removeEventListener = function (type, listener) {
    if (type !== 'abort') return;
    this._listeners = this._listeners.filter(function (item) { return item !== listener; });
  };
  AbortSignal.prototype.dispatchEvent = function (event) {
    if (!event || event.type !== 'abort') return true;
    for (var i = 0; i < this._listeners.length; i++) {
      try { this._listeners[i].call(this, event); } catch (error) {}
    }
    return true;
  };
  AbortSignal.prototype.throwIfAborted = function () {
    if (this.aborted) throw this.reason || new Error('The operation was aborted');
  };
  AbortSignal.timeout = function () {
    // Native requests use the host deadline; a JS timeout signal is accepted
    // for compatibility but cannot interrupt a blocking socket.
    return new AbortSignal();
  };
  AbortSignal.abort = function (reason) {
    var signal = new AbortSignal();
    signal.aborted = true;
    signal.reason = reason;
    return signal;
  };
  var AbortController = function () { this.signal = new AbortSignal(); };
  AbortController.prototype.abort = function (reason) {
    if (this.signal.aborted) return;
    this.signal.aborted = true;
    this.signal.reason = reason;
    this.signal.dispatchEvent({ type: 'abort', target: this.signal });
  };
  globalThis.AbortSignal = AbortSignal;
  globalThis.AbortController = AbortController;

  var __b64_chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/';
  globalThis.btoa = function (input) {
    var str = String(input);
    var output = '';
    for (var i = 0; i < str.length; i += 3) {
      var a = str.charCodeAt(i);
      var b = i + 1 < str.length ? str.charCodeAt(i + 1) : 0;
      var c = i + 2 < str.length ? str.charCodeAt(i + 2) : 0;
      if (a > 255 || b > 255 || c > 255) throw new Error('InvalidCharacterError');
      output += __b64_chars.charAt(a >> 2);
      output += __b64_chars.charAt(((a & 3) << 4) | (b >> 4));
      output += i + 1 < str.length ? __b64_chars.charAt(((b & 15) << 2) | (c >> 6)) : '=';
      output += i + 2 < str.length ? __b64_chars.charAt(c & 63) : '=';
    }
    return output;
  };
  globalThis.atob = function (input) {
    var str = String(input).replace(/[\t\n\f\r ]/g, '').replace(/=+$/, '');
    if (str.length % 4 === 1) throw new Error('InvalidCharacterError');
    var output = '';
    var bits = 0;
    var accumulator = 0;
    for (var i = 0; i < str.length; i++) {
      var value = __b64_chars.indexOf(str.charAt(i));
      if (value < 0) continue;
      accumulator = (accumulator << 6) | value;
      bits += 6;
      if (bits >= 8) {
        bits -= 8;
        output += String.fromCharCode((accumulator >> bits) & 255);
      }
    }
    return output;
  };

  function __bytes_to_base64(bytes) {
    var binary = '';
    for (var i = 0; i < bytes.length; i += 8192) {
      binary += String.fromCharCode.apply(null, bytes.subarray(i, i + 8192));
    }
    return globalThis.btoa(binary);
  }
  function __base64_to_bytes(value) {
    var binary = globalThis.atob(value || '');
    var bytes = new Uint8Array(binary.length);
    for (var i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    return bytes;
  }
  function __bytes_to_hex(bytes) {
    var out = '';
    for (var i = 0; i < bytes.length; i++) {
      var part = bytes[i].toString(16);
      out += part.length < 2 ? '0' + part : part;
    }
    return out;
  }
  function __hex_to_bytes(hex) {
    hex = String(hex || '').replace(/[^0-9a-fA-F]/g, '');
    if (hex.length % 2) hex = '0' + hex;
    var bytes = new Uint8Array(hex.length / 2);
    for (var i = 0; i < hex.length; i += 2) bytes[i / 2] = parseInt(hex.substring(i, i + 2), 16);
    return bytes;
  }
  function __to_bytes(value) {
    if (!value) return new Uint8Array(0);
    if (value instanceof Uint8Array) return value;
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    if (typeof ArrayBuffer !== 'undefined' && ArrayBuffer.isView && ArrayBuffer.isView(value)) {
      return new Uint8Array(value.buffer, value.byteOffset || 0, value.byteLength);
    }
    if (Array.isArray(value)) return new Uint8Array(value);
    if (typeof value === 'string') return new globalThis.TextEncoder().encode(value);
    if (typeof value.length === 'number') return new Uint8Array(Array.prototype.slice.call(value));
    return new Uint8Array(0);
  }
  function __copy_bytes(value) {
    var bytes = __to_bytes(value);
    var copy = new Uint8Array(bytes.length);
    copy.set(bytes);
    return copy;
  }
  function __concat_bytes() {
    var total = 0;
    var parts = [];
    for (var i = 0; i < arguments.length; i++) {
      var part = __to_bytes(arguments[i]);
      parts.push(part);
      total += part.length;
    }
    var output = new Uint8Array(total);
    var offset = 0;
    for (var j = 0; j < parts.length; j++) {
      output.set(parts[j], offset);
      offset += parts[j].length;
    }
    return output;
  }

  globalThis.URLSearchParams = function (init) {
    this._entries = [];
    var self = this;
    if (typeof init === 'string') {
      String(init).replace(/^\?/, '').split('&').forEach(function (pair) {
        if (!pair) return;
        var index = pair.indexOf('=');
        var key = index < 0 ? pair : pair.slice(0, index);
        var value = index < 0 ? '' : pair.slice(index + 1);
        try {
          self._entries.push([
            decodeURIComponent(key.replace(/\+/g, ' ')),
            decodeURIComponent(value.replace(/\+/g, ' '))
          ]);
        } catch (error) {}
      });
    } else if (init && typeof init === 'object') {
      if (Array.isArray(init)) {
        init.forEach(function (pair) {
          if (pair && pair.length >= 2) self._entries.push([String(pair[0]), String(pair[1])]);
        });
      } else {
        Object.keys(init).forEach(function (key) { self._entries.push([String(key), String(init[key])]); });
      }
    }
  };
  globalThis.URLSearchParams.prototype.append = function (key, value) { this._entries.push([String(key), String(value)]); };
  globalThis.URLSearchParams.prototype.delete = function (key) {
    this._entries = this._entries.filter(function (pair) { return pair[0] !== String(key); });
  };
  globalThis.URLSearchParams.prototype.get = function (key) {
    var found = this._entries.find(function (pair) { return pair[0] === String(key); });
    return found ? found[1] : null;
  };
  globalThis.URLSearchParams.prototype.getAll = function (key) {
    return this._entries.filter(function (pair) { return pair[0] === String(key); }).map(function (pair) { return pair[1]; });
  };
  globalThis.URLSearchParams.prototype.has = function (key) {
    return this._entries.some(function (pair) { return pair[0] === String(key); });
  };
  globalThis.URLSearchParams.prototype.set = function (key, value) {
    var found = false;
    var output = [];
    for (var i = 0; i < this._entries.length; i++) {
      if (this._entries[i][0] === String(key)) {
        if (!found) output.push([String(key), String(value)]);
        found = true;
      } else output.push(this._entries[i]);
    }
    if (!found) output.push([String(key), String(value)]);
    this._entries = output;
  };
  globalThis.URLSearchParams.prototype.sort = function () {
    this._entries.sort(function (left, right) { return left[0].localeCompare(right[0]); });
  };
  globalThis.URLSearchParams.prototype.toString = function () {
    return this._entries.map(function (pair) {
      return encodeURIComponent(pair[0]) + '=' + encodeURIComponent(pair[1]);
    }).join('&');
  };
  globalThis.URLSearchParams.prototype.forEach = function (callback, thisArg) {
    var self = this;
    this._entries.slice().forEach(function (pair) { callback.call(thisArg, pair[1], pair[0], self); });
  };
  globalThis.URLSearchParams.prototype.entries = function () { return this._entries.slice()[Symbol.iterator](); };
  globalThis.URLSearchParams.prototype.keys = function () {
    return this._entries.map(function (pair) { return pair[0]; })[Symbol.iterator]();
  };
  globalThis.URLSearchParams.prototype.values = function () {
    return this._entries.map(function (pair) { return pair[1]; })[Symbol.iterator]();
  };
  globalThis.URLSearchParams.prototype[Symbol.iterator] = globalThis.URLSearchParams.prototype.entries;

  globalThis.URL = function (input, base) {
    var raw = String(input);
    if (base && !/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(raw)) {
      var baseText = typeof base === 'string' ? base : base.href;
      try {
        var baseUrl = new globalThis.URL(baseText);
        raw = baseUrl._resolve(raw);
      } catch (error) {
        raw = baseText.replace(/\/[^/]*$/, '/') + raw;
      }
    }
    var parsed;
    try { parsed = JSON.parse(__senzo_parse_url(raw)); } catch (error) { throw new TypeError('Invalid URL'); }
    if (!parsed || !parsed.href) throw new TypeError('Invalid URL');
    this.href = parsed.href;
    this.protocol = parsed.protocol;
    this.username = parsed.username || '';
    this.password = parsed.password || '';
    this.host = parsed.host;
    this.hostname = parsed.hostname;
    this.port = parsed.port;
    this.pathname = parsed.pathname;
    this.search = parsed.search;
    this.hash = parsed.hash;
    this.origin = parsed.origin;
    this.searchParams = new globalThis.URLSearchParams(parsed.search || '');
  };
  globalThis.URL.prototype._resolve = function (relative) {
    if (relative.charAt(0) === '#') return this.href.replace(/#.*$/, '') + relative;
    if (relative.charAt(0) === '?') return this.href.replace(/[?#].*$/, '') + relative;
    if (relative.charAt(0) === '/') return this.origin + relative;
    var path = this.pathname.replace(/[^/]*$/, '') + relative;
    var parts = path.split('/');
    var normalized = [];
    parts.forEach(function (part) {
      if (part === '..') normalized.pop();
      else if (part !== '.') normalized.push(part);
    });
    return this.origin + normalized.join('/');
  };
  globalThis.URL.prototype.toString = function () {
    var search = this.searchParams.toString();
    if (search) search = "?" + search;
    return this.origin + this.pathname + search + this.hash;
  };
  globalThis.URL.prototype.toJSON = function () { return this.href; };
  globalThis.URL.canParse = function (value, base) {
    try { new globalThis.URL(value, base); return true; } catch (error) { return false; }
  };

  globalThis.TextEncoder = function () {};
  globalThis.TextEncoder.prototype.encode = function (value) {
    return __hex_to_bytes(__senzo_text_encode(String(value)));
  };
  globalThis.TextDecoder = function () {};
  globalThis.TextDecoder.prototype.decode = function (value) {
    return __senzo_text_decode(__bytes_to_hex(__to_bytes(value)));
  };

  function __normalize_headers(headers) {
    var output = {};
    if (!headers) return output;
    if (typeof headers.forEach === 'function') {
      headers.forEach(function (value, key) { output[String(key).toLowerCase()] = String(value); });
      return output;
    }
    if (Array.isArray(headers)) {
      headers.forEach(function (pair) {
        if (pair && pair.length >= 2) output[String(pair[0]).toLowerCase()] = String(pair[1]);
      });
      return output;
    }
    Object.keys(headers).forEach(function (key) { output[String(key).toLowerCase()] = String(headers[key]); });
    return output;
  }
  function __normalize_body(body) {
    if (body == null) return { kind: 'none', value: '' };
    if (typeof body === 'string') return { kind: 'text', value: body };
    if (typeof body === 'object' && body._entries) return { kind: 'text', value: body.toString() };
    var bytes = null;
    if (body instanceof ArrayBuffer) bytes = new Uint8Array(body);
    else if (typeof ArrayBuffer !== 'undefined' && ArrayBuffer.isView && ArrayBuffer.isView(body)) {
      bytes = new Uint8Array(body.buffer, body.byteOffset || 0, body.byteLength);
    }
    if (bytes) return { kind: 'base64', value: __bytes_to_base64(bytes) };
    return { kind: 'text', value: String(body) };
  }

  globalThis.Headers = function (init) { this._headers = __normalize_headers(init); };
  globalThis.Headers.prototype.get = function (name) { return this._headers[String(name).toLowerCase()] || null; };
  globalThis.Headers.prototype.has = function (name) {
    return Object.prototype.hasOwnProperty.call(this._headers, String(name).toLowerCase());
  };
  globalThis.Headers.prototype.set = function (name, value) { this._headers[String(name).toLowerCase()] = String(value); };
  globalThis.Headers.prototype.forEach = function (callback) {
    var self = this;
    Object.keys(this._headers).forEach(function (key) { callback(self._headers[key], key, self); });
  };

  globalThis.fetch = function (input, init) {
    init = init || {};
    var url = typeof input === "string"
      ? input
      : (input && input.href && typeof input.toString === "function" ? input.toString() : String(input));
    var method = String(init.method || 'GET').toUpperCase();
    var headers = __normalize_headers(init.headers);
    var body = __normalize_body(init.body);
    if (init.signal && init.signal.aborted) {
      return Promise.reject(init.signal.reason || new Error('The operation was aborted'));
    }
    var request;
    try {
      request = __senzo_fetch(JSON.stringify({
        url: url,
        method: method,
        headers: headers,
        bodyKind: body.kind,
        body: body.value,
        followRedirects: init.redirect !== 'manual'
      }));
    } catch (error) {
      return Promise.reject(error);
    }
    var parsed;
    try { parsed = JSON.parse(request); } catch (error) { return Promise.reject(new Error('Fetch bridge returned invalid data')); }
    if (parsed.error) return Promise.reject(new Error(parsed.error));
    var bytes = __base64_to_bytes(parsed.bodyBase64 || '');
    var response = {
      ok: !!parsed.ok,
      status: parsed.status || 0,
      statusText: parsed.statusText || '',
      url: parsed.url || url,
      redirected: !!parsed.redirected,
      headers: new globalThis.Headers(parsed.headers || {}),
      text: function () { return Promise.resolve(parsed.body == null ? '' : String(parsed.body)); },
      json: function () {
        try {
          if (parsed.body == null || parsed.body === '') return Promise.resolve(null);
          return Promise.resolve(JSON.parse(parsed.body));
        } catch (error) { return Promise.resolve(null); }
      },
      arrayBuffer: function () {
        var copy = new Uint8Array(bytes.length);
        copy.set(bytes);
        return Promise.resolve(copy.buffer);
      },
      clone: function () { return Object.assign({}, response, { headers: new globalThis.Headers(parsed.headers || {}) }); }
    };
    return Promise.resolve(response);
  };

  // The crypto and buffer layers are separate IIFEs, so the byte helpers
  // must be explicitly shared instead of leaking all module internals.
  globalThis.__to_bytes = __to_bytes;
  globalThis.__copy_bytes = __copy_bytes;
  globalThis.__concat_bytes = __concat_bytes;
  globalThis.__bytes_to_hex = __bytes_to_hex;
  globalThis.__hex_to_bytes = __hex_to_bytes;
  globalThis.__bytes_to_base64 = __bytes_to_base64;
  globalThis.__base64_to_bytes = __base64_to_bytes;
})();
