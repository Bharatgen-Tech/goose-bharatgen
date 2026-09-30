//! Honor a client's information-flow-control "hide" directive.
//!
//! The ACP client owns the policy. It decides which tool results are sensitive,
//! using whatever it knows that the agent does not. There is exactly one thing
//! the client cannot do for itself: it cannot un-see a value for a model that it
//! does not host. That is this module.
//!
//! The contract is one field on the permission response:
//!
//! ```json
//! { "outcome": {...}, "_meta": { "ifc": { "hide": true, "ref": "ifc-ref-1" } } }
//! ```
//!
//! When the client sets it, the real tool result is withheld from the model and
//! replaced by a short placeholder naming an opaque reference. The agent
//! advertises support via `agentCapabilities._meta.goose.ifc.hideDirective` so a
//! client can tell whether hiding is available before it relies on it.
//!
//! What this is not: it does not stop a model from narrating something it
//! already knows, and it does not label the agent's prose. It keeps a specific
//! value out of the context window. Everything else is the client's job.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Pending and completed hide directives for the life of the process.
///
/// Keyed by session so two concurrent sessions cannot see each other's values.
#[derive(Default)]
struct HideStore {
    /// `(session, tool_request_id)` the client asked us to hide, awaiting the result.
    pending: HashMap<(String, String), String>,
    /// `(session, ref)` to the withheld text.
    hidden: HashMap<(String, String), String>,
}

fn store() -> &'static Mutex<HideStore> {
    static STORE: OnceLock<Mutex<HideStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HideStore::default()))
}

/// Read `_meta.ifc.hide` from a permission response, returning the ref to use.
pub fn parse_hide_directive(
    meta: Option<&serde_json::Map<String, serde_json::Value>>,
) -> Option<String> {
    let ifc = meta?.get("ifc")?;
    if ifc.get("hide")?.as_bool() != Some(true) {
        return None;
    }
    Some(ifc.get("ref")?.as_str()?.to_string())
}

/// Record that the result of `request_id` must be withheld once it arrives.
pub fn mark_pending(session_id: &str, request_id: &str, hide_ref: &str) {
    store().lock().expect("ifc store poisoned").pending.insert(
        (session_id.to_string(), request_id.to_string()),
        hide_ref.to_string(),
    );
}

/// The text the model sees in place of a withheld result.
fn placeholder(hide_ref: &str) -> String {
    format!(
        "[withheld by information-flow control] The result is not in your context. \
         Refer to it as \"{hide_ref}\"; passing that string to a later tool call \
         substitutes the real value. Do not guess the contents."
    )
}

/// Swap a withheld tool result for its placeholder.
///
/// Called on every finished tool call; a no-op unless the client asked for this
/// one to be hidden. Errors pass through untouched: a failure has no sensitive
/// value to withhold, and replacing it would only confuse the model.
pub fn apply_hide(
    session_id: &str,
    request_id: &str,
    output: Result<rmcp::model::CallToolResult, rmcp::ErrorData>,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    let hide_ref = {
        let mut guard = store().lock().expect("ifc store poisoned");
        guard
            .pending
            .remove(&(session_id.to_string(), request_id.to_string()))
    };
    let Some(hide_ref) = hide_ref else {
        return output;
    };
    let Ok(result) = output else { return output };
    if result.is_error.unwrap_or(false) {
        return Ok(result);
    }

    let text = result
        .content
        .iter()
        .filter_map(|block| block.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n");

    store()
        .lock()
        .expect("ifc store poisoned")
        .hidden
        .insert((session_id.to_string(), hide_ref.clone()), text);

    tracing::info!(
        session_id = %session_id,
        request_id = %request_id,
        hide_ref = %hide_ref,
        "withheld tool result from model context"
    );

    Ok(rmcp::model::CallToolResult::success(vec![
        rmcp::model::ContentBlock::text(placeholder(&hide_ref)),
    ]))
}

/// Substitute real values for any refs the model passed back, before the tool runs.
///
/// This is what makes a withheld value still usable: the model hands back the
/// ref it was given and the tool receives the real text, without the model ever
/// having read it. Unknown refs are left alone so the tool, not this module,
/// decides what to do with them.
pub fn resolve_refs(
    session_id: &str,
    arguments: &mut Option<serde_json::Map<String, serde_json::Value>>,
) {
    let Some(map) = arguments.take() else { return };

    let hidden: Vec<(String, String)> = {
        let guard = store().lock().expect("ifc store poisoned");
        guard
            .hidden
            .iter()
            .filter(|((session, _), _)| session == session_id)
            .map(|((_, r), text)| (r.clone(), text.clone()))
            .collect()
    };

    let mut value = serde_json::Value::Object(map);
    if !hidden.is_empty() {
        let mut count = 0usize;
        substitute(&hidden, &mut value, &mut count);
        if count > 0 {
            tracing::info!(
                session_id = %session_id,
                "resolved {count} withheld reference(s) into tool arguments"
            );
        }
    }
    *arguments = match value {
        serde_json::Value::Object(map) => Some(map),
        _ => None,
    };
}

fn substitute(hidden: &[(String, String)], value: &mut serde_json::Value, count: &mut usize) {
    match value {
        serde_json::Value::String(text) => {
            // Exact match only. Substring replacement would rewrite any string
            // that merely mentions a reference — including the placeholder we
            // put in the model's context, which names the reference by
            // construction. That turned the "this was withheld" notice into a
            // verbatim disclosure of the value it was withholding.
            for (hide_ref, real) in hidden {
                if text.trim() == hide_ref.as_str() {
                    *text = real.clone();
                    *count += 1;
                    break;
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                substitute(hidden, item, count);
            }
        }
        serde_json::Value::Object(map) => {
            for (_, item) in map.iter_mut() {
                substitute(hidden, item, count);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::{CallToolResult, ContentBlock};

    fn meta(json: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        json.as_object().unwrap().clone()
    }

    #[test]
    fn parses_a_hide_directive() {
        let m = meta(serde_json::json!({ "ifc": { "hide": true, "ref": "ifc-ref-7" } }));
        assert_eq!(parse_hide_directive(Some(&m)).as_deref(), Some("ifc-ref-7"));

        // Absent, false, and malformed directives all mean "do not hide".
        assert_eq!(parse_hide_directive(None), None);
        let off = meta(serde_json::json!({ "ifc": { "hide": false, "ref": "x" } }));
        assert_eq!(parse_hide_directive(Some(&off)), None);
        let noref = meta(serde_json::json!({ "ifc": { "hide": true } }));
        assert_eq!(parse_hide_directive(Some(&noref)), None);
    }

    #[test]
    fn withholds_a_result_then_resolves_the_ref_back() {
        let session = "session-withhold";
        mark_pending(session, "req-1", "ifc-ref-1");

        let real = "the launch code is 1234";
        let hidden = apply_hide(
            session,
            "req-1",
            Ok(CallToolResult::success(vec![ContentBlock::text(real)])),
        )
        .unwrap();

        // The model gets a placeholder naming the ref, never the value.
        let shown = hidden.content[0].as_text().unwrap().text.clone();
        assert!(shown.contains("ifc-ref-1"), "placeholder names the ref");
        assert!(
            !shown.contains("1234"),
            "placeholder must not leak the value"
        );

        // Handing the ref back as an argument value yields the real value.
        let mut args = Some(
            serde_json::json!({ "message": "ifc-ref-1" })
                .as_object()
                .unwrap()
                .clone(),
        );
        resolve_refs(session, &mut args);
        assert_eq!(args.unwrap()["message"].as_str().unwrap(), real);
    }

    /// The bug this guards against: resolution used substring replacement, so
    /// any string mentioning a reference got rewritten — including the
    /// placeholder itself, which names the reference. The notice saying "this
    /// was withheld" ended up quoting the withheld value verbatim.
    #[test]
    fn resolution_never_rewrites_the_placeholder() {
        let session = "session-placeholder";
        let real = "the launch code is 4815-1623-42";
        mark_pending(session, "req-1", "ifc-ref-1");
        let hidden = apply_hide(
            session,
            "req-1",
            Ok(CallToolResult::success(vec![ContentBlock::text(real)])),
        )
        .unwrap();
        let placeholder = hidden.content[0].as_text().unwrap().text.clone();
        assert!(
            !placeholder.contains("4815"),
            "placeholder leaked the value"
        );

        // Echoing the placeholder back must not resolve anything: it mentions
        // the reference but is not the reference.
        let mut args = Some(
            serde_json::json!({ "message": placeholder.clone() })
                .as_object()
                .unwrap()
                .clone(),
        );
        resolve_refs(session, &mut args);
        let after = args.unwrap()["message"].as_str().unwrap().to_string();
        assert_eq!(after, placeholder, "placeholder was rewritten");
        assert!(!after.contains("4815"), "resolution leaked the value");
    }

    /// Prose that merely mentions a reference is left alone; only a value that
    /// *is* the reference resolves.
    #[test]
    fn only_an_exact_reference_resolves() {
        let session = "session-exact";
        mark_pending(session, "req-1", "ifc-ref-1");
        apply_hide(
            session,
            "req-1",
            Ok(CallToolResult::success(vec![ContentBlock::text("SECRET")])),
        )
        .unwrap();

        let mut args = Some(
            serde_json::json!({
                "prose": "I hold ifc-ref-1 but will not publish it",
                "exact": "ifc-ref-1",
                "padded": "  ifc-ref-1  ",
            })
            .as_object()
            .unwrap()
            .clone(),
        );
        resolve_refs(session, &mut args);
        let args = args.unwrap();
        assert_eq!(
            args["prose"].as_str().unwrap(),
            "I hold ifc-ref-1 but will not publish it",
            "prose must not be rewritten"
        );
        assert_eq!(args["exact"].as_str().unwrap(), "SECRET");
        assert_eq!(args["padded"].as_str().unwrap(), "SECRET");
    }

    #[test]
    fn leaves_unmarked_results_and_foreign_refs_alone() {
        // No pending directive: the result passes through verbatim.
        let untouched = apply_hide(
            "session-a",
            "req-none",
            Ok(CallToolResult::success(vec![ContentBlock::text("plain")])),
        )
        .unwrap();
        assert_eq!(untouched.content[0].as_text().unwrap().text, "plain");

        // A ref minted in one session is meaningless in another.
        mark_pending("session-a", "req-2", "ifc-ref-2");
        apply_hide(
            "session-a",
            "req-2",
            Ok(CallToolResult::success(vec![ContentBlock::text("secret")])),
        )
        .unwrap();

        let mut args = Some(
            serde_json::json!({ "m": "ifc-ref-2" })
                .as_object()
                .unwrap()
                .clone(),
        );
        resolve_refs("session-b", &mut args);
        assert_eq!(args.unwrap()["m"].as_str().unwrap(), "ifc-ref-2");
    }
}
