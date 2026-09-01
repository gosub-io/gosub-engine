// Environment shim for running WPT .any.js tests in a bare V8 context.
// Loaded before testharness.js; testharness detects this bare global as its
// ShellTestEnvironment. The native half lives on globalThis.__gosub__ (set up
// from Rust) and is consumed and removed here.
"use strict";

var self = globalThis;

// testharness picks its environment from `'document' in globalThis`, so this
// alias does not flip it out of shell mode
globalThis.window = globalThis;

// The .any.js wrapper normally provides this; every accessor answers "plain shell".
globalThis.GLOBAL = {
    isWindow: function () { return false; },
    isWorker: function () { return false; },
    isShadowRealm: function () { return false; },
};

// testharness derives default test names from location.pathname.
if (typeof globalThis.location === "undefined") {
    var testName = typeof globalThis.__gosub_test_name === "string" ? globalThis.__gosub_test_name : "untitled";
    globalThis.location = {
        href: "http://web-platform.test/" + testName,
        pathname: "/" + testName,
        search: "",
        hash: "",
    };
}

// Deferred-callback queue standing in for real timers. Callbacks run when the
// Rust driver calls __drainTimers() after the test file has executed; delays
// are ignored, order is preserved.
(function () {
    var queue = [];
    var nextId = 1;

    globalThis.setTimeout = function (cb, _delay) {
        var args = Array.prototype.slice.call(arguments, 2);
        queue.push({ id: nextId, cb: cb, args: args });
        return nextId++;
    };

    globalThis.clearTimeout = function (id) {
        for (var i = 0; i < queue.length; i++) {
            if (queue[i].id === id) {
                queue.splice(i, 1);
                return;
            }
        }
    };

    globalThis.__drainTimers = function () {
        var ran = 0;
        while (queue.length > 0) {
            ran++;
            if (ran > 100000) {
                throw new Error("__drainTimers: runaway timer loop");
            }
            var t = queue.shift();
            if (typeof t.cb === "function") {
                t.cb.apply(undefined, t.args);
            }
        }
        return ran;
    };
})();

// JS stand-in for DOMException until the native binding exists. Close enough
// for assert_throws_dom: correct name/message/code and the legacy constants on
// both the constructor and the prototype.
if (typeof globalThis.DOMException === "undefined") {
    (function () {
        // Null prototype: a name like "constructor" must miss, not find
        // Object.prototype members
        var LEGACY_CODES = Object.assign(Object.create(null), {
            IndexSizeError: 1,
            HierarchyRequestError: 3,
            WrongDocumentError: 4,
            InvalidCharacterError: 5,
            NoModificationAllowedError: 7,
            NotFoundError: 8,
            NotSupportedError: 9,
            InUseAttributeError: 10,
            InvalidStateError: 11,
            SyntaxError: 12,
            InvalidModificationError: 13,
            NamespaceError: 14,
            InvalidAccessError: 15,
            TypeMismatchError: 17,
            SecurityError: 18,
            NetworkError: 19,
            AbortError: 20,
            URLMismatchError: 21,
            QuotaExceededError: 22,
            TimeoutError: 23,
            InvalidNodeTypeError: 24,
            DataCloneError: 25,
        });
        var CONSTANTS = {
            INDEX_SIZE_ERR: 1, DOMSTRING_SIZE_ERR: 2, HIERARCHY_REQUEST_ERR: 3,
            WRONG_DOCUMENT_ERR: 4, INVALID_CHARACTER_ERR: 5, NO_DATA_ALLOWED_ERR: 6,
            NO_MODIFICATION_ALLOWED_ERR: 7, NOT_FOUND_ERR: 8, NOT_SUPPORTED_ERR: 9,
            INUSE_ATTRIBUTE_ERR: 10, INVALID_STATE_ERR: 11, SYNTAX_ERR: 12,
            INVALID_MODIFICATION_ERR: 13, NAMESPACE_ERR: 14, INVALID_ACCESS_ERR: 15,
            VALIDATION_ERR: 16, TYPE_MISMATCH_ERR: 17, SECURITY_ERR: 18,
            NETWORK_ERR: 19, ABORT_ERR: 20, URL_MISMATCH_ERR: 21,
            QUOTA_EXCEEDED_ERR: 22, TIMEOUT_ERR: 23, INVALID_NODE_TYPE_ERR: 24,
            DATA_CLONE_ERR: 25,
        };

        class DOMException extends Error {
            #brand = true;

            constructor(message, name) {
                super(message === undefined ? "" : String(message));
                this.name = name === undefined ? "Error" : String(name);
            }

            get code() {
                // WebIDL branding: invoking the getter on a non-instance
                // (e.g. the prototype itself) must throw
                if (!(#brand in this)) {
                    throw new TypeError("Illegal invocation");
                }
                return LEGACY_CODES[this.name] || 0;
            }
        }

        // WebIDL accessors are enumerable, unlike class getters — the record
        // conversion in URLSearchParams(init) relies on hitting this getter
        var codeDescriptor = Object.getOwnPropertyDescriptor(DOMException.prototype, "code");
        Object.defineProperty(DOMException.prototype, "code", {
            get: codeDescriptor.get,
            enumerable: true,
            configurable: true,
        });

        Object.keys(CONSTANTS).forEach(function (key) {
            var descriptor = {
                value: CONSTANTS[key],
                writable: false,
                enumerable: true,
                configurable: false,
            };
            Object.defineProperty(DOMException, key, descriptor);
            Object.defineProperty(DOMException.prototype, key, descriptor);
        });

        globalThis.DOMException = DOMException;
    })();
}

// No longer just a DOMException name: its own interface with quota/requested
// members defaulting to null (testharness checks their presence with `in`)
if (typeof globalThis.QuotaExceededError === "undefined") {
    globalThis.QuotaExceededError = class QuotaExceededError extends globalThis.DOMException {
        constructor(message = "", options = {}) {
            super(message, "QuotaExceededError");
            var opts = options === null || options === undefined ? {} : options;
            this.quota = opts.quota === undefined || opts.quota === null ? null : Number(opts.quota);
            this.requested = opts.requested === undefined || opts.requested === null ? null : Number(opts.requested);
        }
    };
}

// Bridge the native bindings onto the global, translating Rust DomException
// errors (thrown as plain Errors with a "Name: message" text) back into real
// DOMExceptions.
(function () {
    var native = globalThis.__gosub__;
    delete globalThis.__gosub__;

    // Native error classes take precedence; anything else that looks like an
    // error name ("InvalidCharacterError: ...") becomes a DOMException.
    var NATIVE_ERRORS = {
        RangeError: RangeError,
        TypeError: TypeError,
        SyntaxError: SyntaxError,
        ReferenceError: ReferenceError,
    };

    function rethrow(e) {
        var text = e instanceof Error ? String(e.message) : String(e);
        var m = /^([A-Za-z]+Error): ?([\s\S]*)$/.exec(text);
        if (m !== null) {
            if (NATIVE_ERRORS[m[1]] !== undefined) {
                throw new NATIVE_ERRORS[m[1]](m[2]);
            }
            if (m[1] === "QuotaExceededError") {
                throw new globalThis.QuotaExceededError(m[2]);
            }
            throw new globalThis.DOMException(m[2], m[1]);
        }
        throw e;
    }

    // %IteratorPrototype%, reached through any built-in iterator
    var iteratorPrototypeBase = Object.getPrototypeOf(Object.getPrototypeOf([][Symbol.iterator]()));

    // A WebIDL default-iterator prototype: inherits %IteratorPrototype% (which
    // supplies @@iterator), with `next` as an enumerable data property.
    // entryAt(id, i) returns a JSON-encoded [key, value] or null.
    function makeIteratorPrototype(entryAt) {
        var proto = Object.create(iteratorPrototypeBase);
        proto.next = function () {
            var pair = JSON.parse(entryAt(this.__ownerId, this.__index));
            if (pair === null) {
                return { done: true, value: undefined };
            }
            this.__index++;
            var value = this.__kind === "entries" ? pair : this.__kind === "keys" ? pair[0] : pair[1];
            return { done: false, value: value };
        };
        return proto;
    }

    function createIterator(proto, ownerId, kind) {
        var it = Object.create(proto);
        Object.defineProperty(it, "__ownerId", { value: ownerId, writable: false });
        Object.defineProperty(it, "__kind", { value: kind, writable: false });
        Object.defineProperty(it, "__index", { value: 0, writable: true });
        return it;
    }

    // WebIDL record<K, V> conversion: own keys in order, per-key descriptor
    // check, key conversion BEFORE [[Get]] (symbol keys throw in the key
    // converter), all before the caller applies any pair.
    function convertRecord(init, convertKey, convertValue) {
        var result = [];
        var keys = Reflect.ownKeys(init);
        for (var i = 0; i < keys.length; i++) {
            var desc = Object.getOwnPropertyDescriptor(init, keys[i]);
            if (desc !== undefined && desc.enumerable === true) {
                var typedKey = convertKey(keys[i]);
                result.push([typedKey, convertValue(init[keys[i]])]);
            }
        }
        return result;
    }

    // WebIDL USVString conversion: symbols throw; lone surrogates are
    // replaced natively when the string crosses into Rust
    function toUSVString(v) {
        if (typeof v === "symbol") {
            throw new TypeError("Cannot convert a Symbol value to a string");
        }
        return String(v);
    }


    // BufferSource → Uint8Array view. A detached buffer reads as empty, per
    // WebIDL's "get a copy of the bytes" (it can detach during options
    // conversion, which runs first).
    function bytesOf(input) {
        if (input === undefined) {
            return new Uint8Array(0);
        }
        if (input instanceof ArrayBuffer ||
            (typeof SharedArrayBuffer !== "undefined" && input instanceof SharedArrayBuffer)) {
            try {
                return new Uint8Array(input);
            } catch (e) {
                return new Uint8Array(0);
            }
        }
        if (ArrayBuffer.isView(input)) {
            try {
                return new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
            } catch (e) {
                return new Uint8Array(0);
            }
        }
        throw new TypeError("input is not a BufferSource");
    }

    // Bytes cross the native boundary as "binary strings" (one code point in
    // U+0000..=U+00FF per byte).
    function toBinaryString(input) {
        var bytes = bytesOf(input);
        var parts = [];
        var CHUNK = 0x2000;
        for (var i = 0; i < bytes.length; i += CHUNK) {
            parts.push(String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK)));
        }
        return parts.join("");
    }

    globalThis.btoa = function btoa(data) {
        try {
            return native.btoa(String(data));
        } catch (e) {
            rethrow(e);
        }
    };

    globalThis.atob = function atob(data) {
        try {
            return native.atob(String(data));
        } catch (e) {
            rethrow(e);
        }
    };

    globalThis.TextEncoder = class TextEncoder {
        get encoding() {
            return "utf-8";
        }

        encode(input) {
            // String() is the USVString conversion: as_string on the native
            // side replaces lone surrogates with U+FFFD, as the spec requires.
            var s = input === undefined ? "" : String(input);
            var bin;
            try {
                bin = native.teEncode(s);
            } catch (e) {
                rethrow(e);
            }
            var out = new Uint8Array(bin.length);
            for (var i = 0; i < bin.length; i++) {
                out[i] = bin.charCodeAt(i);
            }
            return out;
        }
    };

    globalThis.TextDecoder = class TextDecoder {
        constructor(label, options) {
            var l = label === undefined ? "utf-8" : String(label);
            var opts = options === undefined || options === null ? {} : options;
            var fatal = !!opts.fatal;
            var ignoreBOM = !!opts.ignoreBOM;

            var id;
            try {
                id = native.tdNew(l, fatal ? 1 : 0, ignoreBOM ? 1 : 0);
            } catch (e) {
                rethrow(e);
            }

            Object.defineProperty(this, "__id", { value: id, enumerable: false });
            Object.defineProperty(this, "__encoding", { value: native.tdEncoding(id), enumerable: false });
            Object.defineProperty(this, "__fatal", { value: fatal, enumerable: false });
            Object.defineProperty(this, "__ignoreBOM", { value: ignoreBOM, enumerable: false });
        }

        get encoding() {
            return this.__encoding;
        }

        get fatal() {
            return this.__fatal;
        }

        get ignoreBOM() {
            return this.__ignoreBOM;
        }

        decode(input, options) {
            var stream = !!(options !== undefined && options !== null && options.stream);
            var bin = toBinaryString(input);
            try {
                return native.tdDecode(this.__id, bin, stream ? 1 : 0);
            } catch (e) {
                rethrow(e);
            }
        }
    };

    globalThis.URLSearchParams = class URLSearchParams {
        constructor(init) {
            var id;
            if (init !== undefined && init !== null && (typeof init === "object" || typeof init === "function")) {
                if (typeof init[Symbol.iterator] === "function") {
                    // sequence<sequence<USVString>>: validate all pairs before mutating
                    var pairs = [];
                    for (var item of init) {
                        var pair = Array.from(item);
                        if (pair.length !== 2) {
                            throw new TypeError("URLSearchParams: each init entry must be a [name, value] pair");
                        }
                        pairs.push(pair);
                    }
                    id = native.spNew("");
                    for (var p of pairs) {
                        native.spAppend(id, String(p[0]), String(p[1]));
                    }
                } else {
                    // record<USVString, USVString>: set-or-append, because two
                    // JS keys can collapse into one after USVString conversion
                    // (lone surrogates become U+FFFD)
                    var recordPairs = convertRecord(init, toUSVString, toUSVString);
                    id = native.spNew("");
                    for (var rp of recordPairs) {
                        native.spSet(id, rp[0], rp[1]);
                    }
                }
            } else {
                var s = init === undefined ? "" : toUSVString(init);
                if (s.charAt(0) === "?") {
                    s = s.slice(1);
                }
                id = native.spNew(s);
            }
            Object.defineProperty(this, "__id", { value: id });
        }

        append(name, value) {
            if (arguments.length < 2) {
                throw new TypeError("append requires 2 arguments");
            }
            native.spAppend(this.__id, toUSVString(name), toUSVString(value));
        }

        delete(name, value) {
            if (arguments.length < 1) {
                throw new TypeError("delete requires 1 argument");
            }
            native.spDelete(this.__id, toUSVString(name), value === undefined ? 0 : 1, value === undefined ? "" : toUSVString(value));
        }

        get(name) {
            if (arguments.length < 1) {
                throw new TypeError("get requires 1 argument");
            }
            return JSON.parse(native.spGet(this.__id, toUSVString(name)));
        }

        getAll(name) {
            if (arguments.length < 1) {
                throw new TypeError("getAll requires 1 argument");
            }
            return JSON.parse(native.spGetAll(this.__id, toUSVString(name)));
        }

        has(name, value) {
            if (arguments.length < 1) {
                throw new TypeError("has requires 1 argument");
            }
            return native.spHas(this.__id, toUSVString(name), value === undefined ? 0 : 1, value === undefined ? "" : toUSVString(value)) === 1;
        }

        set(name, value) {
            if (arguments.length < 2) {
                throw new TypeError("set requires 2 arguments");
            }
            native.spSet(this.__id, toUSVString(name), toUSVString(value));
        }

        sort() {
            native.spSort(this.__id);
        }

        get size() {
            return native.spSize(this.__id);
        }

        toString() {
            return native.spToString(this.__id);
        }

        entries() {
            return createIterator(searchParamsIteratorPrototype, this.__id, "entries");
        }

        keys() {
            return createIterator(searchParamsIteratorPrototype, this.__id, "keys");
        }

        values() {
            return createIterator(searchParamsIteratorPrototype, this.__id, "values");
        }

        forEach(callback, thisArg) {
            if (typeof callback !== "function") {
                throw new TypeError("forEach: callback is not a function");
            }
            for (var i = 0; ; i++) {
                var pair = JSON.parse(native.spEntryAt(this.__id, i));
                if (pair === null) {
                    break;
                }
                callback.call(thisArg, pair[1], pair[0], this);
            }
        }
    };

    var searchParamsIteratorPrototype = makeIteratorPrototype(function (id, i) {
        return native.spEntryAt(id, i);
    });

    Object.defineProperty(globalThis.URLSearchParams.prototype, Symbol.iterator, {
        value: globalThis.URLSearchParams.prototype.entries,
        writable: true,
        enumerable: false,
        configurable: true,
    });

    globalThis.URL = class URL {
        constructor(url, base) {
            if (arguments.length < 1) {
                throw new TypeError("URL constructor requires an argument");
            }
            var id;
            try {
                id = base === undefined
                    ? native.urlNew(String(url), 0, "")
                    : native.urlNew(String(url), 1, String(base));
            } catch (e) {
                rethrow(e);
            }
            Object.defineProperty(this, "__id", { value: id });
        }

        static parse(url, base) {
            try {
                return base === undefined ? new URL(url) : new URL(url, base);
            } catch (e) {
                return null;
            }
        }

        static canParse(url, base) {
            try {
                void (base === undefined ? new URL(url) : new URL(url, base));
                return true;
            } catch (e) {
                return false;
            }
        }

        get href() { return native.urlGet(this.__id, "href"); }
        set href(v) {
            try {
                native.urlSet(this.__id, "href", String(v));
            } catch (e) {
                rethrow(e);
            }
        }

        toString() { return this.href; }
        toJSON() { return this.href; }

        get origin() { return native.urlGet(this.__id, "origin"); }

        get protocol() { return native.urlGet(this.__id, "protocol"); }
        set protocol(v) { native.urlSet(this.__id, "protocol", String(v)); }

        get username() { return native.urlGet(this.__id, "username"); }
        set username(v) { native.urlSet(this.__id, "username", String(v)); }

        get password() { return native.urlGet(this.__id, "password"); }
        set password(v) { native.urlSet(this.__id, "password", String(v)); }

        get host() { return native.urlGet(this.__id, "host"); }
        set host(v) { native.urlSet(this.__id, "host", String(v)); }

        get hostname() { return native.urlGet(this.__id, "hostname"); }
        set hostname(v) { native.urlSet(this.__id, "hostname", String(v)); }

        get port() { return native.urlGet(this.__id, "port"); }
        set port(v) { native.urlSet(this.__id, "port", String(v)); }

        get pathname() { return native.urlGet(this.__id, "pathname"); }
        set pathname(v) { native.urlSet(this.__id, "pathname", String(v)); }

        get search() { return native.urlGet(this.__id, "search"); }
        set search(v) { native.urlSet(this.__id, "search", String(v)); }

        get hash() { return native.urlGet(this.__id, "hash"); }
        set hash(v) { native.urlSet(this.__id, "hash", String(v)); }

        get searchParams() {
            if (this.__sp === undefined) {
                var spId = native.urlSearchParamsId(this.__id);
                var sp = Object.create(globalThis.URLSearchParams.prototype);
                Object.defineProperty(sp, "__id", { value: spId });
                Object.defineProperty(this, "__sp", { value: sp });
            }
            return this.__sp;
        }
    };

    // WebIDL ByteString conversion: symbols throw, other values stringify,
    // code units above 255 are a TypeError
    function toByteString(v) {
        if (typeof v === "symbol") {
            throw new TypeError("Cannot convert a Symbol value to a string");
        }
        var s = String(v);
        for (var i = 0; i < s.length; i++) {
            if (s.charCodeAt(i) > 255) {
                throw new TypeError("Cannot convert to ByteString: character at index " + i + " is above U+00FF");
            }
        }
        return s;
    }

    globalThis.Headers = class Headers {
        constructor(init) {
            Object.defineProperty(this, "__id", { value: native.hdrNew() });
            if (init === undefined) {
                return;
            }
            if (init === null || (typeof init !== "object" && typeof init !== "function")) {
                throw new TypeError("Headers init must be a sequence or record");
            }
            if (typeof init[Symbol.iterator] === "function") {
                // sequence<sequence<ByteString>>: validate all pairs first
                var pairs = [];
                for (var item of init) {
                    var pair = Array.from(item);
                    if (pair.length !== 2) {
                        throw new TypeError("Headers init: each entry must be a [name, value] pair");
                    }
                    pairs.push(pair);
                }
                for (var p of pairs) {
                    this.append(p[0], p[1]);
                }
            } else {
                // record<ByteString, ByteString>
                var recordPairs = convertRecord(init, toByteString, toByteString);
                for (var rp of recordPairs) {
                    this.append(rp[0], rp[1]);
                }
            }
        }

        append(name, value) {
            if (arguments.length < 2) {
                throw new TypeError("append requires 2 arguments");
            }
            try {
                native.hdrAppend(this.__id, toByteString(name), toByteString(value));
            } catch (e) {
                rethrow(e);
            }
        }

        delete(name) {
            if (arguments.length < 1) {
                throw new TypeError("delete requires 1 argument");
            }
            try {
                native.hdrDelete(this.__id, toByteString(name));
            } catch (e) {
                rethrow(e);
            }
        }

        get(name) {
            if (arguments.length < 1) {
                throw new TypeError("get requires 1 argument");
            }
            try {
                return JSON.parse(native.hdrGet(this.__id, toByteString(name)));
            } catch (e) {
                rethrow(e);
            }
        }

        getSetCookie() {
            return JSON.parse(native.hdrGetSetCookie(this.__id));
        }

        has(name) {
            if (arguments.length < 1) {
                throw new TypeError("has requires 1 argument");
            }
            try {
                return native.hdrHas(this.__id, toByteString(name)) === 1;
            } catch (e) {
                rethrow(e);
            }
        }

        set(name, value) {
            if (arguments.length < 2) {
                throw new TypeError("set requires 2 arguments");
            }
            try {
                native.hdrSet(this.__id, toByteString(name), toByteString(value));
            } catch (e) {
                rethrow(e);
            }
        }

        entries() {
            return createIterator(headersIteratorPrototype, this.__id, "entries");
        }

        keys() {
            return createIterator(headersIteratorPrototype, this.__id, "keys");
        }

        values() {
            return createIterator(headersIteratorPrototype, this.__id, "values");
        }

        forEach(callback, thisArg) {
            if (typeof callback !== "function") {
                throw new TypeError("forEach: callback is not a function");
            }
            for (var i = 0; ; i++) {
                var pair = JSON.parse(native.hdrEntryAt(this.__id, i));
                if (pair === null) {
                    break;
                }
                callback.call(thisArg, pair[1], pair[0], this);
            }
        }
    };

    var headersIteratorPrototype = makeIteratorPrototype(function (id, i) {
        return native.hdrEntryAt(id, i);
    });

    Object.defineProperty(globalThis.Headers.prototype, Symbol.iterator, {
        value: globalThis.Headers.prototype.entries,
        writable: true,
        enumerable: false,
        configurable: true,
    });

    // console namespace per WebIDL: methods are own properties, the
    // [[Prototype]] is an empty object whose prototype is %Object.prototype%,
    // and the global slot is non-enumerable. Replaces V8's builtin console.
    (function () {
        var consoleObj = Object.create(Object.create(Object.prototype));

        // WebIDL DOMString conversion: symbols throw, objects go via ToString
        function toDOMString(v) {
            if (typeof v === "symbol") {
                throw new TypeError("Cannot convert a Symbol value to a string");
            }
            return String(v);
        }

        // Log arguments are "any": symbols stringify instead of throwing
        function stringifyArg(v) {
            return typeof v === "symbol" ? v.toString() : String(v);
        }

        function callNative(method, args) {
            native.consoleCall(method, JSON.stringify(args));
        }

        ["log", "debug", "info", "warn", "error", "trace", "dirxml", "group", "groupCollapsed"].forEach(function (m) {
            consoleObj[m] = function () {
                var out = [];
                for (var i = 0; i < arguments.length; i++) {
                    out.push(stringifyArg(arguments[i]));
                }
                callNative(m, out);
            };
        });

        ["count", "countReset", "time", "timeEnd"].forEach(function (m) {
            consoleObj[m] = function (label) {
                callNative(m, [arguments.length === 0 ? "default" : toDOMString(label)]);
            };
        });

        consoleObj.timeLog = function (label) {
            var args = [arguments.length === 0 ? "default" : toDOMString(label)];
            for (var i = 1; i < arguments.length; i++) {
                args.push(stringifyArg(arguments[i]));
            }
            callNative("timeLog", args);
        };

        consoleObj.assert = function (condition) {
            var args = [!!condition];
            for (var i = 1; i < arguments.length; i++) {
                args.push(stringifyArg(arguments[i]));
            }
            callNative("assert", args);
        };

        consoleObj.dir = function (item) {
            callNative("dir", arguments.length === 0 ? [] : [stringifyArg(item)]);
        };

        consoleObj.table = function (data) {
            callNative("table", arguments.length === 0 ? [] : [stringifyArg(data)]);
        };

        consoleObj.groupEnd = function () {
            callNative("groupEnd", []);
        };

        consoleObj.clear = function () {
            callNative("clear", []);
        };

        Object.defineProperty(consoleObj, Symbol.toStringTag, {
            value: "console",
            writable: false,
            enumerable: false,
            configurable: true,
        });

        Object.defineProperty(globalThis, "console", {
            value: consoleObj,
            writable: true,
            enumerable: false,
            configurable: true,
        });
    })();

    // Event / CustomEvent / EventTarget / AbortController / AbortSignal.
    // Listener lists and the signal graph live in Rust (et*/sig* natives);
    // JS keeps callbacks, reasons and event objects, keyed by numeric id.
    (function () {
        var stateMap = new WeakMap();

        function st(ev) {
            var s = stateMap.get(ev);
            if (s === undefined) {
                throw new TypeError("Illegal invocation");
            }
            return s;
        }

        // isTrusted is [LegacyUnforgeable]: an own accessor whose getter is
        // one shared function across all Event instances
        function isTrustedGetter() {
            return st(this).trusted;
        }

        function setCanceled(s) {
            if (s.cancelable && !s.inPassive) {
                s.canceled = true;
            }
        }

        // WebIDL DOMString conversion: symbols throw instead of stringifying
        function toDOMString(v) {
            if (typeof v === "symbol") {
                throw new TypeError("Cannot convert a Symbol value to a string");
            }
            return String(v);
        }

        // WebIDL dictionary conversion: declared members only, read via
        // [[Get]] in lexicographic order (callers pass `names` pre-sorted)
        function readMembers(init, names, out) {
            if (init === undefined || init === null) {
                return out;
            }
            if (typeof init !== "object" && typeof init !== "function") {
                throw new TypeError("options is not an object");
            }
            for (var i = 0; i < names.length; i++) {
                var value = init[names[i]];
                if (value !== undefined) {
                    out[names[i]] = value;
                }
            }
            return out;
        }

        class Event {
            constructor(type, eventInitDict) {
                if (arguments.length < 1) {
                    throw new TypeError("Event constructor requires at least 1 argument");
                }
                var t = toDOMString(type);
                var raw = readMembers(eventInitDict, ["bubbles", "cancelable", "composed"], {});
                stateMap.set(this, {
                    type: t,
                    bubbles: !!raw.bubbles,
                    cancelable: !!raw.cancelable,
                    composed: !!raw.composed,
                    target: null,
                    currentTarget: null,
                    eventPhase: 0,
                    canceled: false,
                    stopProp: false,
                    stopImmediate: false,
                    dispatching: false,
                    inPassive: false,
                    trusted: false,
                    timeStamp: Date.now(),
                });
                Object.defineProperty(this, "isTrusted", {
                    get: isTrustedGetter,
                    enumerable: true,
                    configurable: false,
                });
            }

            get type() { return st(this).type; }
            get target() { return st(this).target; }
            get srcElement() { return st(this).target; }
            get currentTarget() { return st(this).currentTarget; }
            get eventPhase() { return st(this).eventPhase; }
            get bubbles() { return st(this).bubbles; }
            get cancelable() { return st(this).cancelable; }
            get composed() { return st(this).composed; }
            get defaultPrevented() { return st(this).canceled; }
            get timeStamp() { return st(this).timeStamp; }

            get returnValue() { return !st(this).canceled; }
            set returnValue(v) {
                if (v === false) {
                    setCanceled(st(this));
                }
            }

            get cancelBubble() { return st(this).stopProp; }
            set cancelBubble(v) {
                if (v) {
                    st(this).stopProp = true;
                }
            }

            composedPath() {
                var s = st(this);
                return s.dispatching && s.currentTarget !== null ? [s.currentTarget] : [];
            }

            stopPropagation() { st(this).stopProp = true; }

            stopImmediatePropagation() {
                var s = st(this);
                s.stopProp = true;
                s.stopImmediate = true;
            }

            preventDefault() { setCanceled(st(this)); }

            initEvent(type, bubbles, cancelable) {
                var s = st(this);
                if (s.dispatching) {
                    return;
                }
                s.trusted = false;
                s.target = null;
                s.canceled = false;
                s.stopProp = false;
                s.stopImmediate = false;
                s.type = toDOMString(type);
                s.bubbles = !!bubbles;
                s.cancelable = !!cancelable;
            }
        }

        var PHASES = { NONE: 0, CAPTURING_PHASE: 1, AT_TARGET: 2, BUBBLING_PHASE: 3 };
        Object.keys(PHASES).forEach(function (name) {
            var descriptor = { value: PHASES[name], writable: false, enumerable: true, configurable: false };
            Object.defineProperty(Event, name, descriptor);
            Object.defineProperty(Event.prototype, name, descriptor);
        });

        globalThis.Event = Event;

        globalThis.CustomEvent = class CustomEvent extends Event {
            constructor(type, eventInitDict) {
                if (arguments.length < 1) {
                    throw new TypeError("CustomEvent constructor requires at least 1 argument");
                }
                super(type, eventInitDict);
                var raw = readMembers(eventInitDict, ["detail"], {});
                st(this).detail = raw.detail === undefined ? null : raw.detail;
            }

            get detail() { return st(this).detail; }

            initCustomEvent(type, bubbles, cancelable, detail) {
                var s = st(this);
                this.initEvent(type, bubbles, cancelable);
                if (!s.dispatching) {
                    s.detail = detail === undefined ? null : detail;
                }
            }
        };

        // Callbacks referenced by numeric key on both sides of the boundary.
        // The id map holds strong references: acceptable for a per-test context.
        var callbackIds = new WeakMap();
        var callbacksById = new Map();
        var nextCallbackId = 1;

        function callbackKey(cb) {
            var id = callbackIds.get(cb);
            if (id === undefined) {
                id = nextCallbackId++;
                callbackIds.set(cb, id);
                callbacksById.set(id, cb);
            }
            return id;
        }

        var targetIds = new WeakMap();

        function targetIdOf(t) {
            var id = targetIds.get(t);
            if (id === undefined) {
                throw new TypeError("Illegal invocation");
            }
            return id;
        }

        function dispatchOn(targetObj, event) {
            var tid = targetIdOf(targetObj);
            var s = st(event);
            s.dispatching = true;
            s.target = targetObj;
            s.currentTarget = targetObj;
            s.eventPhase = 2; // AT_TARGET: a plain EventTarget has no tree to propagate through
            var ids = JSON.parse(native.etSnapshot(tid, s.type));
            for (var i = 0; i < ids.length; i++) {
                if (s.stopImmediate) {
                    break;
                }
                var info = JSON.parse(native.etBeginInvoke(tid, ids[i]));
                if (info === null) {
                    continue; // removed since the snapshot
                }
                var cb = callbacksById.get(info.cb);
                s.inPassive = !!info.passive;
                try {
                    if (typeof cb === "function") {
                        cb.call(targetObj, event);
                    } else if (cb !== undefined && cb !== null && typeof cb.handleEvent === "function") {
                        cb.handleEvent(event);
                    }
                } catch (e) {
                    // "Report the exception": dispatch continues past a throwing listener
                    try {
                        console.error("uncaught exception in event listener: " + (e && e.message ? e.message : String(e)));
                    } catch (ignored) { /* console must never break dispatch */ }
                } finally {
                    s.inPassive = false;
                }
            }
            s.eventPhase = 0;
            s.currentTarget = null;
            s.stopProp = false;
            s.stopImmediate = false;
            s.dispatching = false;
            return !s.canceled;
        }

        class EventTarget {
            constructor() {
                targetIds.set(this, native.etNewTarget());
            }

            addEventListener(type, callback) {
                var tid = targetIdOf(this);
                var t = toDOMString(type);
                var options = arguments[2];

                // (AddEventListenerOptions or boolean): objects are the dict,
                // anything else is the boolean capture branch
                var capture = false;
                var once = false;
                var passive = false;
                var signal = null;
                if (options !== undefined && options !== null && (typeof options === "object" || typeof options === "function")) {
                    var rawCapture = options.capture;
                    if (rawCapture !== undefined) {
                        capture = !!rawCapture;
                    }
                    var rawOnce = options.once;
                    if (rawOnce !== undefined) {
                        once = !!rawOnce;
                    }
                    var rawPassive = options.passive;
                    if (rawPassive !== undefined) {
                        passive = !!rawPassive;
                    }
                    var rawSignal = options.signal;
                    if (rawSignal !== undefined) {
                        if (!(rawSignal instanceof AbortSignal)) {
                            throw new TypeError("signal must be an AbortSignal object");
                        }
                        signal = rawSignal;
                    }
                } else {
                    capture = !!options;
                }

                if (callback === undefined || callback === null) {
                    return;
                }
                if (typeof callback !== "function" && typeof callback !== "object") {
                    throw new TypeError("callback must be an object or a function");
                }
                if (signal !== null && signal.aborted) {
                    return;
                }
                var lid = native.etAdd(tid, t, callbackKey(callback), capture ? 1 : 0, passive ? 1 : 0, once ? 1 : 0);
                if (lid !== 0 && signal !== null) {
                    native.sigLink(signalIdOf(signal), tid, lid);
                }
            }

            removeEventListener(type, callback) {
                var tid = targetIdOf(this);
                var t = toDOMString(type);
                var options = arguments[2];

                // EventListenerOptions has only `capture` — passive and the
                // rest must not be read here
                var capture = false;
                if (options !== undefined && options !== null && (typeof options === "object" || typeof options === "function")) {
                    var rawCapture = options.capture;
                    if (rawCapture !== undefined) {
                        capture = !!rawCapture;
                    }
                } else {
                    capture = !!options;
                }

                if (callback === undefined || callback === null) {
                    return;
                }
                var id = callbackIds.get(callback);
                if (id === undefined) {
                    return; // never registered anywhere: nothing to remove
                }
                native.etRemove(tid, t, id, capture ? 1 : 0);
            }

            dispatchEvent(event) {
                if (!(event instanceof Event)) {
                    throw new TypeError("dispatchEvent expects an Event");
                }
                var s = st(event);
                if (s.dispatching) {
                    throw new globalThis.DOMException("The event is already being dispatched", "InvalidStateError");
                }
                s.trusted = false;
                return dispatchOn(this, event);
            }
        }

        globalThis.EventTarget = EventTarget;

        // AbortSignal / AbortController. Reasons stay JS values; the aborted
        // flag and dependent ordering live in the native graph.
        var signalIds = new WeakMap();
        var signalsByNativeId = new Map();
        var signalReasons = new WeakMap();
        var onabortHandlers = new WeakMap();
        var onabortWrappers = new WeakMap();
        var allowSignalConstruction = false;

        function signalIdOf(s) {
            var id = signalIds.get(s);
            if (id === undefined) {
                throw new TypeError("Illegal invocation");
            }
            return id;
        }

        function createSignalObject(nativeId) {
            allowSignalConstruction = true;
            var s;
            try {
                s = new AbortSignal();
            } finally {
                allowSignalConstruction = false;
            }
            signalIds.set(s, nativeId);
            signalsByNativeId.set(nativeId, s);
            return s;
        }

        // The whole cascade is marked aborted and gets its reason before any
        // abort event fires, then events fire in the native-provided order
        function abortTheSignal(signalObj, reason) {
            var order = JSON.parse(native.sigAbort(signalIdOf(signalObj)));
            for (var i = 0; i < order.length; i++) {
                var obj = signalsByNativeId.get(order[i]);
                if (obj !== undefined) {
                    signalReasons.set(obj, reason);
                }
            }
            for (var j = 0; j < order.length; j++) {
                var target = signalsByNativeId.get(order[j]);
                if (target !== undefined) {
                    var ev = new Event("abort");
                    st(ev).trusted = true; // fired by the implementation, not script
                    dispatchOn(target, ev);
                }
            }
        }

        class AbortSignal extends EventTarget {
            constructor() {
                if (!allowSignalConstruction) {
                    throw new TypeError("Illegal constructor");
                }
                super();
            }

            get aborted() {
                return native.sigAborted(signalIdOf(this)) === 1;
            }

            get reason() {
                signalIdOf(this); // brand check
                return signalReasons.get(this);
            }

            get onabort() {
                var handler = onabortHandlers.get(this);
                return handler === undefined ? null : handler;
            }

            set onabort(value) {
                if (typeof value === "function" || (typeof value === "object" && value !== null)) {
                    if (!onabortWrappers.has(this)) {
                        var self_ = this;
                        var wrapper = function (e) {
                            var handler = onabortHandlers.get(self_);
                            if (typeof handler === "function") {
                                handler.call(self_, e);
                            }
                        };
                        onabortWrappers.set(this, wrapper);
                        this.addEventListener("abort", wrapper);
                    }
                    onabortHandlers.set(this, value);
                } else {
                    onabortHandlers.delete(this);
                }
            }

            throwIfAborted() {
                if (this.aborted) {
                    throw this.reason;
                }
            }

            static abort(reason) {
                var s = createSignalObject(native.sigNew());
                native.sigAbort(signalIds.get(s)); // born aborted, no listeners to notify
                signalReasons.set(
                    s,
                    reason === undefined ? new globalThis.DOMException("signal is aborted without reason", "AbortError") : reason
                );
                return s;
            }

            static timeout(ms) {
                // [EnforceRange] unsigned long long
                var raw = Number(ms);
                if (!Number.isFinite(raw) || Math.trunc(raw) < 0 || Math.trunc(raw) > 18446744073709551615) {
                    throw new TypeError("milliseconds is out of range for unsigned long long");
                }
                var delay = Math.trunc(raw);
                var s = createSignalObject(native.sigNew());
                setTimeout(function () {
                    abortTheSignal(s, new globalThis.DOMException("signal timed out", "TimeoutError"));
                }, delay);
                return s;
            }

            static any(signals) {
                var sourceIds = [];
                for (var item of signals) {
                    if (!(item instanceof AbortSignal)) {
                        throw new TypeError("AbortSignal.any expects a sequence of AbortSignal objects");
                    }
                    sourceIds.push(signalIdOf(item));
                }
                var res = JSON.parse(native.sigNewDependent(JSON.stringify(sourceIds)));
                var s = createSignalObject(res.id);
                if (res.abortedFrom !== null) {
                    // Born aborted: share the triggering source's reason instance
                    signalReasons.set(s, signalReasons.get(signalsByNativeId.get(res.abortedFrom)));
                }
                return s;
            }
        }

        globalThis.AbortSignal = AbortSignal;

        var controllerSignals = new WeakMap();

        globalThis.AbortController = class AbortController {
            constructor() {
                controllerSignals.set(this, createSignalObject(native.sigNew()));
            }

            get signal() {
                var s = controllerSignals.get(this);
                if (s === undefined) {
                    throw new TypeError("Illegal invocation");
                }
                return s;
            }

            abort(reason) {
                abortTheSignal(
                    this.signal,
                    reason === undefined ? new globalThis.DOMException("signal is aborted without reason", "AbortError") : reason
                );
            }
        };

        // The global object is an EventTarget (window): delegate to a hidden
        // instance so globalThis.addEventListener("x", null) etc. work.
        var globalTarget = new EventTarget();
        globalThis.addEventListener = function addEventListener() {
            return EventTarget.prototype.addEventListener.apply(globalTarget, arguments);
        };
        globalThis.removeEventListener = function removeEventListener() {
            return EventTarget.prototype.removeEventListener.apply(globalTarget, arguments);
        };
        globalThis.dispatchEvent = function dispatchEvent() {
            return EventTarget.prototype.dispatchEvent.apply(globalTarget, arguments);
        };
    })();

    // Web Storage. Strings cross the native boundary JSON-escaped (JS strings
    // can hold lone surrogates, a Rust String can't). Named access goes
    // through a Proxy with WebIDL legacy-platform-object semantics.
    (function () {
        // target/proxy -> native storage id; methods run with this === proxy,
        // traps receive the target, so both are registered
        var storageIds = new WeakMap();

        function idFor(obj) {
            var id = storageIds.get(obj);
            if (id === undefined) {
                throw new TypeError("Illegal invocation");
            }
            return id;
        }

        function enc(v) {
            return JSON.stringify(String(v));
        }

        // Native getters return JSON of (escaped-string | null): one parse
        // yields the escaped form, the second the actual DOMString.
        function unwrapOpt(raw) {
            var escaped = JSON.parse(raw);
            return escaped === null ? null : JSON.parse(escaped);
        }

        class Storage {
            key(n) {
                if (arguments.length < 1) {
                    throw new TypeError("key requires 1 argument");
                }
                return unwrapOpt(native.stKey(idFor(this), n >>> 0));
            }

            getItem(key) {
                if (arguments.length < 1) {
                    throw new TypeError("getItem requires 1 argument");
                }
                return unwrapOpt(native.stGetItem(idFor(this), enc(key)));
            }

            setItem(key, value) {
                if (arguments.length < 2) {
                    throw new TypeError("setItem requires 2 arguments");
                }
                // Convert (and possibly throw) both arguments before mutating
                var k = enc(key);
                var v = enc(value);
                try {
                    native.stSetItem(idFor(this), k, v);
                } catch (e) {
                    rethrow(e);
                }
            }

            removeItem(key) {
                if (arguments.length < 1) {
                    throw new TypeError("removeItem requires 1 argument");
                }
                native.stRemoveItem(idFor(this), enc(key));
            }

            clear() {
                native.stClear(idFor(this));
            }

            get length() {
                return native.stLength(idFor(this));
            }
        }

        globalThis.Storage = Storage;

        function makeStorage() {
            var id = native.stNew();
            var target = Object.create(Storage.prototype);

            var handler = {
                get: function (t, prop, receiver) {
                    if (typeof prop === "symbol" || Reflect.has(t, prop)) {
                        return Reflect.get(t, prop, receiver);
                    }
                    var v = unwrapOpt(native.stGetItem(id, enc(prop)));
                    return v === null ? undefined : v;
                },
                set: function (t, prop, value, receiver) {
                    if (typeof prop === "symbol") {
                        return Reflect.set(t, prop, value, receiver);
                    }
                    var k = enc(prop);
                    var v = enc(value); // may throw (value.toString) before storing
                    try {
                        native.stSetItem(id, k, v);
                    } catch (e) {
                        rethrow(e);
                    }
                    return true;
                },
                has: function (t, prop) {
                    if (typeof prop === "symbol" || Reflect.has(t, prop)) {
                        return Reflect.has(t, prop);
                    }
                    return JSON.parse(native.stGetItem(id, enc(prop))) !== null;
                },
                deleteProperty: function (t, prop) {
                    if (typeof prop === "symbol") {
                        return Reflect.deleteProperty(t, prop);
                    }
                    native.stRemoveItem(id, enc(prop));
                    return true;
                },
                ownKeys: function (t) {
                    var keys = JSON.parse(native.stKeys(id)).map(function (k) {
                        return JSON.parse(k);
                    });
                    // Symbols defined directly on the target must be reported
                    // (a non-configurable one is an ownKeys invariant)
                    return keys.concat(Object.getOwnPropertySymbols(t));
                },
                getOwnPropertyDescriptor: function (t, prop) {
                    if (typeof prop === "symbol") {
                        return Reflect.getOwnPropertyDescriptor(t, prop);
                    }
                    // A named property shadowed by anything on the prototype
                    // chain is not visible as an own property
                    if (Reflect.has(t, prop)) {
                        return undefined;
                    }
                    var v = unwrapOpt(native.stGetItem(id, enc(prop)));
                    if (v === null) {
                        return undefined;
                    }
                    return { value: v, writable: true, enumerable: true, configurable: true };
                },
                defineProperty: function (t, prop, desc) {
                    if (typeof prop === "symbol") {
                        return Reflect.defineProperty(t, prop, desc);
                    }
                    if (!("value" in desc)) {
                        return false;
                    }
                    var k = enc(prop);
                    var v = enc(desc.value);
                    try {
                        native.stSetItem(id, k, v);
                    } catch (e) {
                        rethrow(e);
                    }
                    return true;
                },
            };

            var proxy = new Proxy(target, handler);
            storageIds.set(target, id);
            storageIds.set(proxy, id);
            return proxy;
        }

        Object.defineProperty(globalThis, "localStorage", {
            value: makeStorage(),
            writable: true,
            enumerable: true,
            configurable: true,
        });
        Object.defineProperty(globalThis, "sessionStorage", {
            value: makeStorage(),
            writable: true,
            enumerable: true,
            configurable: true,
        });

        function toNullableString(v) {
            return v === undefined || v === null ? null : String(v);
        }

        function toStorageOrNull(v) {
            if (v === undefined || v === null) {
                return null;
            }
            if (!(v instanceof Storage)) {
                throw new TypeError("storageArea must be a Storage object or null");
            }
            return v;
        }

        globalThis.StorageEvent = class StorageEvent extends globalThis.Event {
            constructor(type, eventInitDict = {}) {
                if (arguments.length < 1) {
                    throw new TypeError("StorageEvent constructor requires at least 1 argument");
                }
                super(type, eventInitDict);
                var init = eventInitDict === null || eventInitDict === undefined ? {} : eventInitDict;
                this.key = toNullableString(init.key);
                this.oldValue = toNullableString(init.oldValue);
                this.newValue = toNullableString(init.newValue);
                // url is USVString (not nullable): null stringifies, undefined defaults
                this.url = init.url === undefined ? "" : String(init.url);
                this.storageArea = toStorageOrNull(init.storageArea);
            }

            // Single declared parameter: .length must be 1
            initStorageEvent(type) {
                if (arguments.length < 1) {
                    throw new TypeError("initStorageEvent requires at least 1 argument");
                }
                // Reinitialize the Event base through initEvent (type/bubbles/
                // cancelable live in Event's internal state, not own props)
                this.initEvent(type, !!arguments[1], !!arguments[2]);
                this.key = toNullableString(arguments[3]);
                this.oldValue = toNullableString(arguments[4]);
                this.newValue = toNullableString(arguments[5]);
                this.url = arguments[6] === undefined ? "" : String(arguments[6]);
                this.storageArea = toStorageOrNull(arguments[7]);
            }
        };
    })();

    // Just enough fetch() for testharness's fetch_json helper: resolves the
    // path against the test file's directory (or the wpt root for /-absolute
    // paths) via a native file read.
    globalThis.fetch = function fetch(resource) {
        try {
            var text = native.readRelative(String(resource));
            return Promise.resolve({
                ok: true,
                status: 200,
                text: function () { return Promise.resolve(text); },
                json: function () { return Promise.resolve(JSON.parse(text)); },
            });
        } catch (e) {
            return Promise.reject(new TypeError("fetch failed: " + (e && e.message ? e.message : e)));
        }
    };
})();
