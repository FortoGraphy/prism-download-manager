//! Envelope rules shared by host and app.
//!
//! * Extension → host: `{"id": string|number, "type": string, "noLaunch"?: bool, ...}`.
//!   The host forwards the whole object unchanged as the body of
//!   `POST /v1/message` (`Content-Type: application/json`, `Authorization: Bearer <token>`).
//! * App → host (HTTP 200): a JSON object `{"ok": true, ...}` or
//!   `{"ok": false, "error": {"code": string, "message": string}}`.
//! * Host → extension: the app's object with `"id"` set to the request id.
//!   Errors produced by the host itself use the same error shape with the
//!   codes below.
//! * Host-local message: `{"id", "type": "hostPing"}` → `{"id", "ok": true, "host": {"version"}}`
//!   (never touches the app).

use serde_json::{Map, Value, json};

/// Error codes produced by the host.
pub mod codes {
    /// Frame was not valid JSON (reply `id` is null).
    pub const BAD_JSON: &str = "bad_json";
    /// JSON is not an object with `id` and string `type`.
    pub const BAD_REQUEST: &str = "bad_request";
    /// The app is not running and `noLaunch` was set.
    pub const APP_NOT_RUNNING: &str = "app_not_running";
    /// The app could not be launched / did not come up within 8 s.
    pub const APP_UNREACHABLE: &str = "app_unreachable";
    /// The app rejected the bearer token even after re-reading ipc.json.
    pub const UNAUTHORIZED: &str = "unauthorized";
    /// The app answered with a non-2xx status.
    pub const APP_ERROR: &str = "app_error";
    /// The app answered 2xx with a body that is not a JSON object.
    pub const BAD_REPLY: &str = "bad_reply";
    /// The app's reply would exceed the 1 MiB host → browser limit.
    pub const REPLY_TOO_LARGE: &str = "reply_too_large";
    /// The app did not answer in time.
    pub const TIMEOUT: &str = "timeout";
    /// The browser sent a frame above 64 MiB.
    pub const TOO_LARGE: &str = "too_large";
}

/// `{"id": id, "ok": false, "error": {"code", "message"}}`.
pub fn error_reply(id: &Value, code: &str, message: impl Into<String>) -> Value {
    json!({ "id": id, "ok": false, "error": { "code": code, "message": message.into() } })
}

/// Puts `id` on the app's reply. Non-object replies are wrapped as `{"ok": true, "data": ...}`.
pub fn attach_id(id: &Value, reply: Value) -> Value {
    let mut obj = match reply {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("ok".into(), Value::Bool(true));
            m.insert("data".into(), other);
            m
        }
    };
    obj.insert("id".into(), id.clone());
    Value::Object(obj)
}

/// Validates an incoming message; returns `(id, type)`.
pub fn validate(msg: &Value) -> Result<(Value, String), (Value, String)> {
    let id = msg.get("id").cloned().unwrap_or(Value::Null);
    if !msg.is_object() {
        return Err((Value::Null, "message must be a JSON object".into()));
    }
    if !(id.is_string() || id.is_number()) {
        return Err((id, "missing \"id\" (string or number)".into()));
    }
    match msg.get("type").and_then(Value::as_str) {
        Some(t) if !t.is_empty() => Ok((id, t.to_string())),
        _ => Err((id, "missing \"type\"".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attach_and_validate() {
        let r = attach_id(&json!("a1"), json!({"ok": true, "x": 1}));
        assert_eq!(r, json!({"ok": true, "x": 1, "id": "a1"}));
        let r = attach_id(&json!(7), json!([1, 2]));
        assert_eq!(r, json!({"ok": true, "data": [1, 2], "id": 7}));
        assert_eq!(
            validate(&json!({"id": 1, "type": "hello"})).unwrap().1,
            "hello"
        );
        assert!(validate(&json!({"type": "hello"})).is_err());
        assert!(validate(&json!({"id": "x"})).is_err());
        assert!(validate(&json!([1])).is_err());
        let e = error_reply(&json!("q"), codes::TIMEOUT, "slow");
        assert_eq!(e["error"]["code"], "timeout");
        assert_eq!(e["ok"], false);
    }
}
