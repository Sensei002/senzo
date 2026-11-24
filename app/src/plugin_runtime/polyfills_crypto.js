// CryptoJS and WebCrypto compatibility for Nuvio providers. All primitives are
// implemented with RustCrypto; this file only handles WordArray/encoding shape.

(function () {
  'use strict';

  var WordArray = {
    init: function (words, sigBytes) {
      this.words = words || [];
      this.sigBytes = sigBytes != null ? sigBytes : this.words.length * 4;
    },
    toString: function (encoder) { return (encoder || CryptoJS.enc.Hex).stringify(this); },
    concat: function (wordArray) {
      this.clamp();
      for (var i = 0; i < wordArray.sigBytes; i++) {
        var byte = (wordArray.words[i >>> 2] >>> (24 - (i % 4) * 8)) & 0xff;
        this.words[(this.sigBytes + i) >>> 2] |= byte << (24 - ((this.sigBytes + i) % 4) * 8);
      }
      this.sigBytes += wordArray.sigBytes;
      return this;
    },
    clamp: function () {
      if (this.sigBytes % 4) this.words[this.sigBytes >>> 2] &= 0xffffffff << (32 - (this.sigBytes % 4) * 8);
      this.words.length = Math.ceil(this.sigBytes / 4);
      return this;
    },
    clone: function () { return __word_array_create(this.words.slice(0), this.sigBytes); }
  };
  function __word_array_create(words, sigBytes) {
    var value = Object.create(WordArray);
    value.init(words, sigBytes);
    return value;
  }
  function __is_word_array(value) {
    return value && typeof value === 'object' && Array.isArray(value.words) && typeof value.sigBytes === 'number';
  }
  function __word_array_to_bytes(value) {
    if (!__is_word_array(value)) return __to_bytes(value);
    var bytes = new Uint8Array(value.sigBytes);
    for (var i = 0; i < value.sigBytes; i++) {
      bytes[i] = (value.words[i >>> 2] >>> (24 - (i % 4) * 8)) & 0xff;
    }
    return bytes;
  }
  function __bytes_to_word_array(bytes) {
    bytes = __to_bytes(bytes);
    var words = [];
    for (var i = 0; i < bytes.length; i++) words[i >>> 2] |= (bytes[i] & 0xff) << (24 - (i % 4) * 8);
    return __word_array_create(words, bytes.length);
  }
  function __hash_name(value) {
    var name = value && value.name ? value.name : value;
    name = String(name || 'SHA-256').toUpperCase().replace(/[^A-Z0-9]/g, '');
    if (name === 'SHA1' || name === 'SHA256' || name === 'SHA384' || name === 'SHA512' || name === 'MD5') return name;
    throw new Error('Unsupported hash algorithm: ' + name);
  }
  function __algorithm_name(value) {
    var name = value && value.name ? value.name : value;
    name = String(name || '').toUpperCase();
    if (name.indexOf('AES-GCM') >= 0) return 'AES-GCM';
    if (name.indexOf('AES-CBC') >= 0) return 'AES-CBC';
    if (name.indexOf('AES-ECB') >= 0) return 'AES-ECB';
    if (name.indexOf('PBKDF2') >= 0) return 'PBKDF2';
    if (name.indexOf('HMAC') >= 0) return 'HMAC';
    return name;
  }
  function __native_digest(hash, data) {
    return __hex_to_bytes(__crypto_digest_hex(__hash_name(hash), __bytes_to_hex(__to_bytes(data))));
  }
  function __native_hmac(hash, key, data) {
    return __hex_to_bytes(__crypto_hmac_hex(
      __hash_name(hash),
      __bytes_to_hex(__to_bytes(key)),
      __bytes_to_hex(__to_bytes(data))
    ));
  }
  function __native_pbkdf2(password, salt, iterations, bits, hash) {
    return __hex_to_bytes(__crypto_pbkdf2_hex(
      __bytes_to_hex(__to_bytes(password)),
      __bytes_to_hex(__to_bytes(salt)),
      iterations,
      bits,
      __hash_name(hash)
    ));
  }
  function __native_aes(encrypt, mode, key, iv, data) {
    var fn = encrypt ? __crypto_aes_encrypt_hex : __crypto_aes_decrypt_hex;
    return __hex_to_bytes(fn(
      mode,
      __bytes_to_hex(__to_bytes(key)),
      __bytes_to_hex(__to_bytes(iv)),
      __bytes_to_hex(__to_bytes(data))
    ));
  }
  function __evp_kdf(password, salt, keySize, ivSize) {
    var output = new Uint8Array(keySize + ivSize);
    var block = new Uint8Array(0);
    var offset = 0;
    while (offset < output.length) {
      block = __native_digest('MD5', __concat_bytes(block, password, salt || new Uint8Array(0)));
      var take = Math.min(block.length, output.length - offset);
      output.set(block.subarray(0, take), offset);
      offset += take;
    }
    return { key: output.subarray(0, keySize), iv: output.subarray(keySize) };
  }
  function __openssl_header() { return new Uint8Array([83, 97, 108, 116, 101, 100, 95, 95]); }
  function __has_openssl_header(bytes) {
    var header = __openssl_header();
    if (!bytes || bytes.length < 16) return false;
    for (var i = 0; i < header.length; i++) if (bytes[i] !== header[i]) return false;
    return true;
  }
  function __cipher_params(ciphertext, key, iv, salt, mode) {
    return {
      ciphertext: __bytes_to_word_array(ciphertext),
      key: key ? __bytes_to_word_array(key) : undefined,
      iv: iv ? __bytes_to_word_array(iv) : undefined,
      salt: salt ? __bytes_to_word_array(salt) : undefined,
      mode: mode,
      toString: function (formatter) { return (formatter || CryptoJS.format.OpenSSL).stringify(this); }
    };
  }
  function __aes_mode(mode, padding) {
    var normalized = __algorithm_name(mode || 'AES-CBC');
    if (padding === CryptoJS.pad.NoPadding || padding === 'NoPadding') normalized += '-NoPadding';
    return normalized;
  }

  var CryptoJS = {
    enc: {
      Hex: {
        stringify: function (value) { return __bytes_to_hex(__word_array_to_bytes(value)); },
        parse: function (value) { return __bytes_to_word_array(__hex_to_bytes(value)); }
      },
      Utf8: {
        stringify: function (value) { return new TextDecoder().decode(__word_array_to_bytes(value)); },
        parse: function (value) { return __bytes_to_word_array(new TextEncoder().encode(String(value))); }
      },
      Latin1: {
        stringify: function (value) {
          var bytes = __word_array_to_bytes(value);
          var output = '';
          for (var i = 0; i < bytes.length; i++) output += String.fromCharCode(bytes[i]);
          return output;
        },
        parse: function (value) {
          value = String(value || '');
          var bytes = new Uint8Array(value.length);
          for (var i = 0; i < value.length; i++) bytes[i] = value.charCodeAt(i) & 0xff;
          return __bytes_to_word_array(bytes);
        }
      },
      Base64: {
        stringify: function (value) {
          var bytes = __word_array_to_bytes(value);
          var binary = '';
          for (var i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
          return btoa(binary);
        },
        parse: function (value) { return __bytes_to_word_array(__base64_to_bytes(String(value || ''))); }
      },
      Base64url: {
        stringify: function (value) {
          return CryptoJS.enc.Base64.stringify(value).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
        },
        parse: function (value) {
          var text = String(value || '').replace(/-/g, '+').replace(/_/g, '/');
          while (text.length % 4) text += '=';
          return CryptoJS.enc.Base64.parse(text);
        }
      }
    },
    lib: {
      WordArray: {
        create: function (words, sigBytes) {
          if (words == null) return __word_array_create([], sigBytes || 0);
          if (__is_word_array(words)) return words.clone();
          if (typeof words === 'string') return CryptoJS.enc.Utf8.parse(words);
          if (words instanceof ArrayBuffer || (typeof ArrayBuffer !== 'undefined' && ArrayBuffer.isView && ArrayBuffer.isView(words))) {
            var bytes = __to_bytes(words);
            return __bytes_to_word_array(sigBytes != null ? bytes.subarray(0, sigBytes) : bytes);
          }
          return __word_array_create(words, sigBytes);
        },
        random: function (size) {
          var bytes = new Uint8Array(size || 0);
          globalThis.crypto.getRandomValues(bytes);
          return __bytes_to_word_array(bytes);
        }
      },
      CipherParams: {
        create: function (params) {
          params = params || {};
          params.toString = params.toString || function (formatter) {
            return (formatter || CryptoJS.format.OpenSSL).stringify(this);
          };
          return params;
        }
      }
    },
    format: {
      OpenSSL: {
        stringify: function (params) {
          var cipher = __word_array_to_bytes(params.ciphertext);
          var output = params.salt
            ? __concat_bytes(__openssl_header(), __word_array_to_bytes(params.salt), cipher)
            : cipher;
          return CryptoJS.enc.Base64.stringify(__bytes_to_word_array(output));
        },
        parse: function (value) {
          var bytes = __word_array_to_bytes(CryptoJS.enc.Base64.parse(value));
          if (__has_openssl_header(bytes)) {
            return CryptoJS.lib.CipherParams.create({
              salt: __bytes_to_word_array(bytes.subarray(8, 16)),
              ciphertext: __bytes_to_word_array(bytes.subarray(16))
            });
          }
          return CryptoJS.lib.CipherParams.create({ ciphertext: __bytes_to_word_array(bytes) });
        }
      }
    },
    mode: { CBC: 'AES-CBC', GCM: 'AES-GCM', ECB: 'AES-ECB' },
    pad: { Pkcs7: 'Pkcs7', NoPadding: 'NoPadding' },
    algo: { MD5: 'MD5', SHA1: 'SHA1', SHA256: 'SHA256', SHA384: 'SHA384', SHA512: 'SHA512', AES: 'AES' },
    MD5: function (message) { return __bytes_to_word_array(__native_digest('MD5', message)); },
    SHA1: function (message) { return __bytes_to_word_array(__native_digest('SHA1', message)); },
    SHA256: function (message) { return __bytes_to_word_array(__native_digest('SHA256', message)); },
    SHA384: function (message) { return __bytes_to_word_array(__native_digest('SHA384', message)); },
    SHA512: function (message) { return __bytes_to_word_array(__native_digest('SHA512', message)); },
    HmacMD5: function (message, key) { return __bytes_to_word_array(__native_hmac('MD5', key, message)); },
    HmacSHA1: function (message, key) { return __bytes_to_word_array(__native_hmac('SHA1', key, message)); },
    HmacSHA256: function (message, key) { return __bytes_to_word_array(__native_hmac('SHA256', key, message)); },
    HmacSHA384: function (message, key) { return __bytes_to_word_array(__native_hmac('SHA384', key, message)); },
    HmacSHA512: function (message, key) { return __bytes_to_word_array(__native_hmac('SHA512', key, message)); },
    PBKDF2: function (password, salt, options) {
      options = options || {};
      var size = (options.keySize || 8) * 32;
      return __bytes_to_word_array(__native_pbkdf2(
        password,
        salt,
        options.iterations || 1000,
        size,
        options.hasher || 'SHA1'
      ));
    },
    AES: {
      encrypt: function (message, key, options) {
        options = options || {};
        var data = __word_array_to_bytes(message);
        var keyBytes;
        var ivBytes;
        var saltBytes;
        if (typeof key === 'string') {
          saltBytes = options.salt ? __word_array_to_bytes(options.salt) : __word_array_to_bytes(CryptoJS.lib.WordArray.random(8));
          var derived = __evp_kdf(new TextEncoder().encode(key), saltBytes, 32, 16);
          keyBytes = derived.key;
          ivBytes = options.iv ? __word_array_to_bytes(options.iv) : derived.iv;
        } else {
          keyBytes = __word_array_to_bytes(key);
          ivBytes = options.iv ? __word_array_to_bytes(options.iv) : new Uint8Array(0);
        }
        var mode = __aes_mode(options.mode || 'AES-CBC', options.padding);
        return __cipher_params(__native_aes(true, mode, keyBytes, ivBytes, data), keyBytes, ivBytes, saltBytes, mode);
      },
      decrypt: function (cipher, key, options) {
        options = options || {};
        var params = typeof cipher === 'string' ? CryptoJS.format.OpenSSL.parse(cipher) : cipher;
        var data = params.ciphertext ? __word_array_to_bytes(params.ciphertext) : __to_bytes(params);
        var keyBytes;
        var ivBytes;
        if (typeof key === 'string') {
          var salt = options.salt
            ? __word_array_to_bytes(options.salt)
            : (params.salt ? __word_array_to_bytes(params.salt) : new Uint8Array(0));
          var derived = __evp_kdf(new TextEncoder().encode(key), salt, 32, 16);
          keyBytes = derived.key;
          ivBytes = options.iv ? __word_array_to_bytes(options.iv) : derived.iv;
        } else {
          keyBytes = __word_array_to_bytes(key);
          ivBytes = options.iv ? __word_array_to_bytes(options.iv) : new Uint8Array(0);
        }
        var mode = __aes_mode(options.mode || 'AES-CBC', options.padding);
        return __bytes_to_word_array(__native_aes(false, mode, keyBytes, ivBytes, data));
      }
    }
  };
  globalThis.CryptoJS = CryptoJS;

  function __crypto_key(type, algorithm, extractable, usages, raw) {
    return {
      type: type,
      extractable: !!extractable,
      algorithm: algorithm,
      usages: usages || [],
      _raw: __copy_bytes(raw)
    };
  }
  function __web_algorithm(value) {
    var name = __algorithm_name(value);
    var output = { name: name };
    if (value && typeof value === 'object' && value.length) output.length = value.length;
    if (value && typeof value === 'object' && value.hash) output.hash = { name: __hash_name(value.hash) };
    return output;
  }

  globalThis.crypto = {
    subtle: {
      digest: function (algorithm, data) {
        return Promise.resolve(__copy_bytes(__native_digest(algorithm, data)).buffer);
      },
      importKey: function (format, data, algorithm, extractable, usages) {
        format = String(format || 'raw').toLowerCase();
        if (format !== 'raw' && format !== 'pkcs8' && format !== 'spki') {
          return Promise.reject(new Error('Unsupported key format: ' + format));
        }
        var type = format === 'spki' ? 'public' : (format === 'pkcs8' ? 'private' : 'secret');
        return Promise.resolve(__crypto_key(type, __web_algorithm(algorithm || {}), extractable, usages, data));
      },
      exportKey: function (format, key) {
        format = String(format || 'raw').toLowerCase();
        if (format !== 'raw' && format !== 'pkcs8' && format !== 'spki') {
          return Promise.reject(new Error('Unsupported key format: ' + format));
        }
        return Promise.resolve(__copy_bytes(key._raw).buffer);
      },
      generateKey: function (algorithm, extractable, usages) {
        var options = __web_algorithm(algorithm || {});
        if (options.name !== 'AES-CBC' && options.name !== 'AES-GCM' && options.name !== 'HMAC') {
          return Promise.reject(new Error('Unsupported generateKey algorithm: ' + options.name));
        }
        var bytes = new Uint8Array((options.length || 256) / 8);
        globalThis.crypto.getRandomValues(bytes);
        return Promise.resolve(__crypto_key('secret', options, extractable, usages, bytes));
      },
      deriveBits: function (params, key, length) {
        if (__algorithm_name(params) !== 'PBKDF2') {
          return Promise.reject(new Error('Only PBKDF2 deriveBits is supported'));
        }
        return Promise.resolve(__copy_bytes(__native_pbkdf2(
          key._raw,
          params.salt,
          params.iterations || 1000,
          length,
          params.hash || 'SHA-256'
        )).buffer);
      },
      deriveKey: function (params, key, algorithm, extractable, usages) {
        var options = __web_algorithm(algorithm || {});
        var length = options.length || 256;
        return globalThis.crypto.subtle.deriveBits(params, key, length).then(function (raw) {
          return __crypto_key('secret', options, extractable, usages, new Uint8Array(raw));
        });
      },
      encrypt: function (params, key, data) {
        var mode = __algorithm_name(params);
        if (mode !== 'AES-CBC' && mode !== 'AES-GCM') {
          return Promise.reject(new Error('Unsupported encrypt algorithm: ' + mode));
        }
        if (mode === 'AES-GCM' && params.tagLength && params.tagLength !== 128) {
          return Promise.reject(new Error('Only 128-bit AES-GCM tags are supported'));
        }
        return Promise.resolve(__copy_bytes(__native_aes(
          true,
          mode,
          key._raw,
          params.iv || new Uint8Array(0),
          data
        )).buffer);
      },
      decrypt: function (params, key, data) {
        var mode = __algorithm_name(params);
        if (mode !== 'AES-CBC' && mode !== 'AES-GCM') {
          return Promise.reject(new Error('Unsupported decrypt algorithm: ' + mode));
        }
        if (mode === 'AES-GCM' && params.tagLength && params.tagLength !== 128) {
          return Promise.reject(new Error('Only 128-bit AES-GCM tags are supported'));
        }
        return Promise.resolve(__copy_bytes(__native_aes(
          false,
          mode,
          key._raw,
          params.iv || new Uint8Array(0),
          data
        )).buffer);
      },
      sign: function (algorithm, key, data) {
        if (__algorithm_name(algorithm || key.algorithm) === 'HMAC' || key.algorithm.name === 'HMAC') {
          var hash = (algorithm && algorithm.hash) || (key.algorithm && key.algorithm.hash) || 'SHA-256';
          return Promise.resolve(__copy_bytes(__native_hmac(hash, key._raw, data)).buffer);
        }
        return Promise.reject(new Error('Only HMAC signatures are supported'));
      },
      verify: function (algorithm, key, signature, data) {
        if (__algorithm_name(algorithm || key.algorithm) === 'HMAC' || key.algorithm.name === 'HMAC') {
          var expected = __native_hmac(
            (algorithm && algorithm.hash) || (key.algorithm && key.algorithm.hash) || 'SHA-256',
            key._raw,
            data
          );
          var actual = __to_bytes(signature);
          if (expected.length !== actual.length) return Promise.resolve(false);
          var difference = 0;
          for (var i = 0; i < expected.length; i++) difference |= expected[i] ^ actual[i];
          return Promise.resolve(difference === 0);
        }
        return Promise.reject(new Error('Only HMAC signatures are supported'));
      }
    },
    getRandomValues: function (array) {
      var length = array.byteLength != null ? array.byteLength : array.length;
      var random = __hex_to_bytes(__crypto_random_hex(length || 0));
      if (array.buffer && array.byteLength != null) {
        new Uint8Array(array.buffer, array.byteOffset || 0, array.byteLength).set(random);
      } else {
        for (var i = 0; i < array.length; i++) array[i] = random[i] || 0;
      }
      return array;
    },
    randomUUID: function () {
      var bytes = new Uint8Array(16);
      globalThis.crypto.getRandomValues(bytes);
      bytes[6] = (bytes[6] & 0x0f) | 0x40;
      bytes[8] = (bytes[8] & 0x3f) | 0x80;
      var hex = __bytes_to_hex(bytes);
      return hex.slice(0, 8) + '-' + hex.slice(8, 12) + '-' + hex.slice(12, 16) + '-' + hex.slice(16, 20) + '-' + hex.slice(20);
    }
  };

  globalThis.WebAssembly = {
    instantiate: function () { return Promise.reject(new Error('WebAssembly providers are not supported by this build')); },
    compile: function () { return Promise.reject(new Error('WebAssembly providers are not supported by this build')); }
  };

  function Buffer(value, encoding) { return Buffer.from(value, encoding); }
  Buffer.from = function (value, encoding) {
    if (typeof value === 'string') {
      encoding = String(encoding || 'utf8').toLowerCase();
      if (encoding === 'hex') return __hex_to_bytes(value);
      if (encoding === 'base64') return __base64_to_bytes(value);
      if (encoding === 'latin1' || encoding === 'binary') {
        var bytes = new Uint8Array(value.length);
        for (var i = 0; i < value.length; i++) bytes[i] = value.charCodeAt(i) & 0xff;
        return bytes;
      }
      return new TextEncoder().encode(value);
    }
    if (value instanceof ArrayBuffer) return new Uint8Array(value);
    if (Array.isArray(value) || (value && typeof value.length === 'number')) return new Uint8Array(value);
    return new Uint8Array(0);
  };
  Buffer.alloc = function (size, fill) {
    var bytes = new Uint8Array(size || 0);
    if (fill != null) bytes.fill(typeof fill === 'number' ? fill : (String(fill).charCodeAt(0) || 0));
    return bytes;
  };
  Buffer.concat = function (list, total) {
    var length = total;
    if (length == null) length = list.reduce(function (sum, item) { return sum + item.length; }, 0);
    var output = new Uint8Array(length);
    var offset = 0;
    list.forEach(function (item) {
      var part = __to_bytes(item);
      var take = Math.min(part.length, length - offset);
      output.set(part.subarray(0, take), offset);
      offset += take;
    });
    return output;
  };
  Buffer.isBuffer = function (value) { return value instanceof Uint8Array; };
  Buffer.byteLength = function (value, encoding) { return Buffer.from(value, encoding).length; };
  Uint8Array.prototype.toString = function (encoding) {
    encoding = String(encoding || 'utf8').toLowerCase();
    if (encoding === 'hex') return __bytes_to_hex(this);
    if (encoding === 'base64') return __bytes_to_base64(this);
    if (encoding === 'latin1' || encoding === 'binary') {
      var output = '';
      for (var i = 0; i < this.length; i++) output += String.fromCharCode(this[i]);
      return output;
    }
    return __senzo_text_decode(__bytes_to_hex(this));
  };
  globalThis.Buffer = Buffer;
})();
