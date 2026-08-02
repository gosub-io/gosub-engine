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
            constructor(message, name) {
                super(message === undefined ? "" : String(message));
                this.name = name === undefined ? "Error" : String(name);
            }

            get code() {
                return LEGACY_CODES[this.name] || 0;
            }
        }

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

    function rethrow(e) {
        var text = e instanceof Error ? String(e.message) : String(e);
        var m = /^([A-Za-z]+Error): ?([\s\S]*)$/.exec(text);
        if (m !== null) {
            throw new globalThis.DOMException(m[2], m[1]);
        }
        throw e;
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
