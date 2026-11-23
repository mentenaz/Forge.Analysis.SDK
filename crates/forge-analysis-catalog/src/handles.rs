//! What the network objects the catalog calls "sources" are, and which methods the platform itself gives them.
//!
//! A call on a network object (`res.json()`, `socket.setNoDelay()`) is ordinary when the platform defines that method for
//! that kind of object. Any other method name is not: whoever answered the request decided what runs. The kind of object
//! decides which names are the platform's, so a `Response` has no `end()` and a socket has no `json()`. Facts about APIs
//! only, like the rest of the catalog: no severity.

/// The kind of network object a source call gives back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HandleKind {
    /// Not known: the combined list of platform methods is used.
    #[default]
    Unknown,
    /// `fetch(..)`: a `Response`.
    Response,
    /// `http.get(..)`, `https.request(..)`: a request or an incoming message.
    Http,
    /// `net.connect(..)`, `tls.connect(..)`, `http2.connect(..)`: a socket or session.
    Socket,
    /// `new WebSocket(..)`.
    WebSocket,
    /// `new XMLHttpRequest()`.
    Xhr,
}

/// The kind of object the catalog entry `id` gives back, for the network sources.
pub fn handle_kind(entry_id: &str) -> HandleKind {
    match entry_id {
        "web.global.fetch" => HandleKind::Response,
        "web.global.websocket" => HandleKind::WebSocket,
        "web.global.xmlhttprequest" => HandleKind::Xhr,
        "node.http.request" | "node.https.request" => HandleKind::Http,
        "node.net.connect" | "node.tls.connect" | "node.http2.connect" => HandleKind::Socket,
        _ => HandleKind::Unknown,
    }
}

/// Every object has these, and so does the promise a source call may still be (`fetch(u).then(..)`).
const OBJECT: &[&str] = &[
    "toString", "valueOf", "toJSON", "toLocaleString", "hasOwnProperty", "isPrototypeOf", "propertyIsEnumerable", "then",
    "catch", "finally",
];

const EMITTER: &[&str] = &[
    "on", "once", "off", "addListener", "removeListener", "removeAllListeners", "emit", "listeners", "listenerCount",
    "prependListener", "prependOnceListener", "eventNames", "setMaxListeners", "getMaxListeners", "rawListeners",
];

const EVENT_TARGET: &[&str] = &["addEventListener", "removeEventListener", "dispatchEvent"];

const RESPONSE: &[&str] = &["json", "text", "blob", "arrayBuffer", "formData", "bytes", "clone"];

const STREAM: &[&str] = &[
    "pipe", "unpipe", "setEncoding", "setTimeout", "pause", "resume", "isPaused", "destroy", "end", "write", "abort", "read",
    "cancel", "cork", "uncork", "destroySoon", "unshift", "wrap", "push", "flushHeaders", "setNoDelay", "setSocketKeepAlive",
    "setHeader", "getHeader", "getHeaders", "getHeaderNames", "hasHeader", "removeHeader", "getRawHeaders",
];

const SOCKET: &[&str] = &[
    "connect", "setKeepAlive", "ref", "unref", "address", "getPeerCertificate", "getCertificate", "getCipher", "getProtocol",
    "getSession", "getTLSTicket", "getX509Certificate", "getEphemeralKeyInfo", "isSessionReused", "renegotiate",
    "setMaxSendFragment", "setServername", "setSession", "enableTrace", "exportKeyingMaterial", "getFinished",
    "getPeerFinished", "getSharedSigalgs", "disableRenegotiation", "resetAndDestroy", "close", "request", "ping",
    "settings", "goaway", "setLocalWindowSize", "altsvc", "origin",
];

const WEBSOCKET: &[&str] = &["send", "close"];

const XHR: &[&str] = &[
    "open", "send", "abort", "setRequestHeader", "getResponseHeader", "getAllResponseHeaders", "overrideMimeType",
];

/// The combined list, for an object whose kind is not known: strings, arrays, maps, promises and every kind above.
const COMBINED_EXTRA: &[&str] = &[
    // strings
    "at", "charAt", "charCodeAt", "codePointAt", "concat", "endsWith", "includes", "indexOf", "lastIndexOf", "localeCompare",
    "match", "matchAll", "normalize", "padEnd", "padStart", "repeat", "replace", "replaceAll", "search", "slice", "split",
    "startsWith", "substring", "substr", "toLowerCase", "toUpperCase", "toLocaleLowerCase", "toLocaleUpperCase", "trim",
    "trimStart", "trimEnd",
    // arrays
    "map", "filter", "forEach", "reduce", "reduceRight", "find", "findIndex", "findLast", "findLastIndex", "some", "every",
    "flat", "flatMap", "join", "pop", "shift", "splice", "sort", "reverse", "fill", "keys", "values", "entries", "copyWithin",
    "toSorted", "toReversed", "toSpliced", "with", "toFixed", "toPrecision",
    // maps, sets, headers, promises
    "get", "set", "has", "add", "delete", "clear", "append", "then", "catch", "finally", "getReader",
];

fn any(lists: &[&[&str]], name: &str) -> bool {
    lists.iter().any(|l| l.contains(&name))
}

/// Is `name` a method the platform defines for an object of this kind?
pub fn is_platform_method(kind: HandleKind, name: &str) -> bool {
    match kind {
        HandleKind::Unknown => any(
            &[OBJECT, EMITTER, EVENT_TARGET, RESPONSE, STREAM, SOCKET, WEBSOCKET, XHR, COMBINED_EXTRA],
            name,
        ),
        HandleKind::Response => any(&[OBJECT, RESPONSE], name),
        HandleKind::Http => any(&[OBJECT, EMITTER, STREAM], name),
        HandleKind::Socket => any(&[OBJECT, EMITTER, STREAM, SOCKET], name),
        HandleKind::WebSocket => any(&[OBJECT, EVENT_TARGET, WEBSOCKET], name),
        HandleKind::Xhr => any(&[OBJECT, EVENT_TARGET, XHR], name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_network_source_in_the_catalog_has_a_kind() {
        let cat = crate::Catalog::node_defaults();
        for e in cat.entries() {
            let source = e.category == forge_analysis_schema::Effect::NetOut && e.role == Some(crate::Role::Source);
            // `dns.lookup` gives back plain data (an address), not a network object: it stays on the combined list.
            let plain_data = ["node.dns.lookup"];
            if source && !plain_data.contains(&e.id.as_str()) {
                assert_ne!(handle_kind(&e.id), HandleKind::Unknown, "{} has no handle kind", e.id);
            }
        }
    }

    #[test]
    fn a_type_only_has_its_own_methods() {
        assert!(is_platform_method(HandleKind::Response, "json"));
        assert!(!is_platform_method(HandleKind::Response, "end"), "a Response has no end()");
        assert!(is_platform_method(HandleKind::Response, "then"), "the promise a source call may still be");
        assert!(!is_platform_method(HandleKind::Response, "send"));
        assert!(is_platform_method(HandleKind::Socket, "setNoDelay"));
        assert!(is_platform_method(HandleKind::Socket, "getPeerCertificate"));
        assert!(!is_platform_method(HandleKind::Socket, "json"));
        assert!(is_platform_method(HandleKind::WebSocket, "send"));
        assert!(!is_platform_method(HandleKind::WebSocket, "open"));
        assert!(is_platform_method(HandleKind::Xhr, "open"));
    }

    #[test]
    fn an_unknown_kind_uses_the_combined_list() {
        for n in ["json", "end", "send", "open", "trim", "map", "on", "setNoDelay"] {
            assert!(is_platform_method(HandleKind::Unknown, n), "{n}");
        }
        assert!(!is_platform_method(HandleKind::Unknown, "scan"));
    }
}
