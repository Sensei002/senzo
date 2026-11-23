// Cheerio-compatible DOM facade and Node module aliases for Nuvio providers.
// Rust owns the HTML tree (html5ever/scraper); this file only translates the
// chainable jQuery subset used by the provider corpus.

(function () {
  'use strict';

  function __cheerio_ids(value) {
    if (typeof value !== 'string') return [];
    try {
      var parsed = JSON.parse(value);
      return Array.isArray(parsed) ? parsed : [];
    } catch (error) {
      return [];
    }
  }
  function __split_contains(selector) {
    var needles = [];
    var cleaned = String(selector || '').replace(/:contains\((?:"([^"]*)"|'([^']*)'|([^)]*))\)/gi, function (_, double, single, plain) {
      needles.push(double != null ? double : (single != null ? single : plain));
      return '';
    });
    cleaned = cleaned.replace(/\s+/g, ' ').trim();
    return { selector: cleaned || '*', needles: needles };
  }
  function __filter_contains(docId, ids, needles) {
    if (!needles.length) return ids;
    return ids.filter(function (id) {
      var text = __cheerio_text(docId, id);
      return needles.every(function (needle) { return text.indexOf(needle) >= 0; });
    });
  }

  function __create_wrapper(docId, ids) {
    var wrapper = {
      _docId: docId,
      _elementIds: ids,
      length: ids.length,
      each: function (callback) {
        for (var i = 0; i < ids.length; i++) {
          var item = __create_wrapper(docId, [ids[i]]);
          callback.call(item, i, item);
        }
        return wrapper;
      },
      find: function (selector) {
        var parts = __split_contains(selector);
        var output = [];
        for (var i = 0; i < ids.length; i++) {
          output = output.concat(__cheerio_ids(__cheerio_find(docId, ids[i], parts.selector)));
        }
        return __create_wrapper(docId, __filter_contains(docId, output, parts.needles));
      },
      children: function (selector) {
        var parts = __split_contains(selector || '*');
        var output = [];
        for (var i = 0; i < ids.length; i++) {
          output = output.concat(__cheerio_ids(__cheerio_children(docId, ids[i], parts.selector)));
        }
        return __create_wrapper(docId, __filter_contains(docId, output, parts.needles));
      },
      text: function () {
        if (!ids.length) return '';
        return ids.map(function (id) { return __cheerio_text(docId, id); }).join('');
      },
      html: function () { return ids.length ? __cheerio_inner_html(docId, ids[0]) : ''; },
      attr: function (name) {
        if (!ids.length) return undefined;
        var value = __cheerio_attr(docId, ids[0], String(name));
        return value === '__UNDEFINED__' ? undefined : value;
      },
      first: function () { return __create_wrapper(docId, ids.length ? [ids[0]] : []); },
      last: function () { return __create_wrapper(docId, ids.length ? [ids[ids.length - 1]] : []); },
      next: function () {
        var output = [];
        for (var i = 0; i < ids.length; i++) {
          var value = __cheerio_next(docId, ids[i]);
          if (value && value !== '__NONE__') output.push(value);
        }
        return __create_wrapper(docId, output);
      },
      prev: function () {
        var output = [];
        for (var i = 0; i < ids.length; i++) {
          var value = __cheerio_prev(docId, ids[i]);
          if (value && value !== '__NONE__') output.push(value);
        }
        return __create_wrapper(docId, output);
      },
      parent: function () {
        var output = [];
        for (var i = 0; i < ids.length; i++) {
          var value = __cheerio_parent(docId, ids[i]);
          if (value && value !== '__NONE__') output.push(value);
        }
        return __create_wrapper(docId, output);
      },
      eq: function (index) {
        var position = index < 0 ? ids.length + index : index;
        return __create_wrapper(docId, position >= 0 && position < ids.length ? [ids[position]] : []);
      },
      get: function (index) {
        if (typeof index === 'number') {
          var position = index < 0 ? ids.length + index : index;
          return position >= 0 && position < ids.length ? __create_wrapper(docId, [ids[position]]) : undefined;
        }
        return ids.map(function (id) { return __create_wrapper(docId, [id]); });
      },
      map: function (callback) {
        var results = [];
        for (var i = 0; i < ids.length; i++) {
          var item = __create_wrapper(docId, [ids[i]]);
          var value = callback.call(item, i, item);
          if (value !== undefined && value !== null) results.push(value);
        }
        return {
          length: results.length,
          get: function (index) { return typeof index === 'number' ? results[index] : results; },
          toArray: function () { return results; }
        };
      },
      filter: function (selectorOrCallback) {
        if (typeof selectorOrCallback === 'function') {
          var kept = [];
          for (var i = 0; i < ids.length; i++) {
            var item = __create_wrapper(docId, [ids[i]]);
            if (selectorOrCallback.call(item, i, item)) kept.push(ids[i]);
          }
          return __create_wrapper(docId, kept);
        }
        var parts = __split_contains(selectorOrCallback);
        var matched = ids.filter(function (id) { return __cheerio_is(docId, id, parts.selector); });
        return __create_wrapper(docId, __filter_contains(docId, matched, parts.needles));
      },
      is: function (selector) {
        var parts = __split_contains(selector);
        for (var i = 0; i < ids.length; i++) {
          if (__cheerio_is(docId, ids[i], parts.selector)
            && __filter_contains(docId, [ids[i]], parts.needles).length) {
            return true;
          }
        }
        return false;
      },
      closest: function (selector) {
        var parts = __split_contains(selector);
        var output = [];
        for (var i = 0; i < ids.length; i++) {
          var value = __cheerio_closest(docId, ids[i], parts.selector);
          if (value && value !== '__NONE__') output.push(value);
        }
        return __create_wrapper(docId, __filter_contains(docId, output, parts.needles));
      },
      toArray: function () { return ids.map(function (id) { return __create_wrapper(docId, [id]); }); },
      remove: function () { return wrapper; },
      addClass: function () { return wrapper; },
      removeClass: function () { return wrapper; }
    };
    return wrapper;
  }

  var cheerio = {
    load: function (html) {
      var docId = __cheerio_load(String(html || ''));
      var $ = function (selector, context) {
        if (selector == null || selector === "") return __create_wrapper(docId, []);
        if (selector && selector._elementIds) return selector;
        var parts = __split_contains(selector);
        var ids;
        if (context && context._elementIds && context._elementIds.length) {
          ids = [];
          context._elementIds.forEach(function (id) {
            ids = ids.concat(__cheerio_ids(__cheerio_find(docId, id, parts.selector)));
          });
        } else {
          ids = __cheerio_ids(__cheerio_select(docId, parts.selector));
        }
        return __create_wrapper(docId, __filter_contains(docId, ids, parts.needles));
      };
      $.html = function (element) {
        if (element && element._elementIds && element._elementIds.length) {
          return __cheerio_html(docId, element._elementIds[0]);
        }
        return __cheerio_html(docId, 0);
      };
      return $;
    }
  };
  globalThis.cheerio = cheerio;

  var __empty_module = {};
  var __util_module = {
    inherits: function (constructor, parent) {
      constructor.prototype = Object.create(parent.prototype);
      constructor.prototype.constructor = constructor;
    },
    format: function () {
      return Array.prototype.map.call(arguments, function (value) { return String(value); }).join(' ');
    },
    inspect: function (value) {
      try { return JSON.stringify(value); } catch (error) { return String(value); }
    },
    isArray: Array.isArray,
    deprecate: function (fn) { return fn; },
    promisify: function (fn) {
      return function () {
        var args = Array.prototype.slice.call(arguments);
        var self = this;
        return new Promise(function (resolve, reject) {
          args.push(function (error, value) { if (error) reject(error); else resolve(value); });
          fn.apply(self, args);
        });
      };
    },
    types: { isNativeError: function (value) { return value instanceof Error; } }
  };
  function __assert(value, message) { if (!value) throw new Error(message || 'Assertion failed'); }
  __assert.ok = __assert;
  __assert.equal = function (left, right, message) { if (left != right) throw new Error(message || 'Assertion failed'); };
  __assert.strictEqual = function (left, right, message) { if (left !== right) throw new Error(message || 'Assertion failed'); };
  __assert.deepEqual = __assert.deepStrictEqual = function (left, right, message) {
    if (JSON.stringify(left) !== JSON.stringify(right)) throw new Error(message || 'Assertion failed');
  };
  __assert.throws = function (fn) {
    try { fn(); } catch (error) { return; }
    throw new Error('Expected function to throw');
  };

  globalThis.require = function (name) {
    name = String(name || '');
    if (name === 'cheerio' || name === 'cheerio-without-node-native' || name === 'react-native-cheerio') return cheerio;
    if (name === 'crypto-js') return globalThis.CryptoJS;
    if (name === 'buffer') return { Buffer: globalThis.Buffer };
    if (name === 'crypto') return globalThis.crypto;
    if (name === 'util') return __util_module;
    if (name === 'util/types') return __util_module.types;
    if (name === 'assert' || name === 'assert/strict') return __assert;
    if (name === 'events') return { EventEmitter: function () {} };
    if (name === 'stream' || name === 'zlib' || name === 'net' || name === 'tls'
      || name === 'http' || name === 'http2' || name === 'async_hooks'
      || name === 'diagnostics_channel' || name === 'timers') {
      return __empty_module;
    }
    throw new Error("Module '" + name + "' is not available");
  };

  if (!Array.prototype.flat) {
    Array.prototype.flat = function (depth) {
      var level = depth === undefined ? 1 : Math.floor(depth);
      if (level < 1) return Array.prototype.slice.call(this);
      var flatten = function (items, remaining) {
        return remaining > 0
          ? items.reduce(function (output, item) {
            return output.concat(Array.isArray(item) ? flatten(item, remaining - 1) : item);
          }, [])
          : items.slice();
      };
      return flatten(this, level);
    };
  }
  if (!Array.prototype.flatMap) {
    Array.prototype.flatMap = function (callback, thisArg) { return this.map(callback, thisArg).flat(); };
  }
  if (!Object.entries) {
    Object.entries = function (object) {
      return Object.keys(object).map(function (key) { return [key, object[key]]; });
    };
  }
  if (!Object.fromEntries) {
    Object.fromEntries = function (entries) {
      var output = {};
      entries.forEach(function (pair) { output[pair[0]] = pair[1]; });
      return output;
    };
  }
  if (!String.prototype.replaceAll) {
    String.prototype.replaceAll = function (search, replacement) {
      if (search instanceof RegExp) {
        if (!search.global) throw new TypeError('replaceAll must be called with a global RegExp');
        return this.replace(search, replacement);
      }
      return this.split(String(search)).join(String(replacement));
    };
  }
  if (!globalThis.structuredClone) {
    globalThis.structuredClone = function (value) { return JSON.parse(JSON.stringify(value)); };
  }
  if (!globalThis.setImmediate) {
    globalThis.setImmediate = function (callback) {
      return globalThis.setTimeout(callback, 0, ...Array.prototype.slice.call(arguments, 1));
    };
  }
  if (!globalThis.clearImmediate) globalThis.clearImmediate = globalThis.clearTimeout;
})();
