// Environment shim for running WPT .any.js tests in a bare V8 context.
// Loaded before testharness.js; testharness detects this bare global as its
// ShellTestEnvironment. The native half lives on globalThis.__gosub__ (set up
// from Rust) and is consumed and removed here.
"use strict";

var self = globalThis;

// .window.js tests address the global as `window`. testharness picks its
// environment from `'document' in globalThis`, so this alias alone does not
// flip it out of shell mode.
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
        var LEGACY_CODES = {
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
        };
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

// QuotaExceededError is no longer just a DOMException name: it is its own
// interface deriving from DOMException, carrying optional quota/requested
// members that default to null (and must exist — testharness checks `in`).
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


    // BufferSource → Uint8Array view, per WebIDL. Construction over a
    // detached buffer throws; WebIDL's "get a copy of the bytes" treats a
    // detached buffer as empty instead (it can detach during options
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
                var s = init === undefined ? "" : String(init);
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
            native.spAppend(this.__id, String(name), String(value));
        }

        delete(name, value) {
            if (arguments.length < 1) {
                throw new TypeError("delete requires 1 argument");
            }
            native.spDelete(this.__id, String(name), value === undefined ? 0 : 1, value === undefined ? "" : String(value));
        }

        get(name) {
            if (arguments.length < 1) {
                throw new TypeError("get requires 1 argument");
            }
            return JSON.parse(native.spGet(this.__id, String(name)));
        }

        getAll(name) {
            if (arguments.length < 1) {
                throw new TypeError("getAll requires 1 argument");
            }
            return JSON.parse(native.spGetAll(this.__id, String(name)));
        }

        has(name, value) {
            if (arguments.length < 1) {
                throw new TypeError("has requires 1 argument");
            }
            return native.spHas(this.__id, String(name), value === undefined ? 0 : 1, value === undefined ? "" : String(value)) === 1;
        }

        set(name, value) {
            if (arguments.length < 2) {
                throw new TypeError("set requires 2 arguments");
            }
            native.spSet(this.__id, String(name), String(value));
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

    // Web Storage. The native Storage lives in Rust; every DOMString crosses
    // the boundary JSON-escaped (JSON.stringify emits ASCII escapes for lone
    // surrogates, which a Rust String could not hold). Named access,
    // enumeration and defineProperty follow WebIDL's legacy-platform-object
    // semantics via a Proxy: string keys map to the storage list, symbols fall
    // through to the target, and prototype members are never shadowed
    // (Storage has no [LegacyOverrideBuiltIns]).
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

        // Minimal Event/StorageEvent, enough for the constructor and
        // initStorageEvent surfaces (no dispatch — EventTarget comes later).
        if (typeof globalThis.Event === "undefined") {
            globalThis.Event = class Event {
                constructor(type, eventInitDict = {}) {
                    if (arguments.length < 1) {
                        throw new TypeError("Event constructor requires at least 1 argument");
                    }
                    var init = eventInitDict === null || eventInitDict === undefined ? {} : eventInitDict;
                    this.type = String(type);
                    this.bubbles = !!init.bubbles;
                    this.cancelable = !!init.cancelable;
                    this.composed = !!init.composed;
                    this.target = null;
                    this.currentTarget = null;
                    this.defaultPrevented = false;
                    this.isTrusted = false;
                    this.timeStamp = 0;
                }

                stopPropagation() {}
                stopImmediatePropagation() {}
                preventDefault() {}
            };
        }

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
                this.type = String(type);
                this.bubbles = !!arguments[1];
                this.cancelable = !!arguments[2];
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
