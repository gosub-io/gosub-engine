// Environment shim for running WPT .any.js tests in a bare V8 context.
// Loaded before testharness.js; testharness detects this bare global as its
// ShellTestEnvironment. The native half lives on globalThis.__gosub__ (set up
// from Rust) and is consumed and removed here.
"use strict";

var self = globalThis;

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
            throw new globalThis.DOMException(m[2], m[1]);
        }
        throw e;
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
                    id = native.spNew("");
                    for (var key of Object.keys(init)) {
                        native.spSet(id, String(key), String(init[key]));
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
            return makeSearchParamsIterator(this, "entries");
        }

        keys() {
            return makeSearchParamsIterator(this, "keys");
        }

        values() {
            return makeSearchParamsIterator(this, "values");
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

    // Live index-based iteration, per the spec's iterator semantics
    function makeSearchParamsIterator(sp, kind) {
        var i = 0;
        var iterator = {
            next: function () {
                var pair = JSON.parse(native.spEntryAt(sp.__id, i));
                if (pair === null) {
                    return { done: true, value: undefined };
                }
                i++;
                var value = kind === "entries" ? pair : kind === "keys" ? pair[0] : pair[1];
                return { done: false, value: value };
            },
        };
        iterator[Symbol.iterator] = function () {
            return iterator;
        };
        return iterator;
    }

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
