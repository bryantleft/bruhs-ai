//! Typing a vault value into a bruh's page.
//!
//! The origin check and the write are one step bound to one document: both
//! run as fixed functions in an isolated world created on the main frame's
//! document, which dies with that document. A page that moved between the
//! check and the write makes the write fail, so nothing is typed there.
//! The value travels only as an argument of the write call, never in
//! source, a reply or an error.

use std::io::Write as _;

use bruhs_models::{
    crew::{
        secret_input_types, stays_out_of_addresses, FILL_FAILED, FILL_GET_FORM, LOGIN_WRONG_ORIGIN,
        SECRET_FIELD_MISMATCH, SIGN_IN_AUTOCOMPLETE, SUBMIT_ONLY_SIGNS_IN,
    },
    local_tools::{cdp_key, ToolReply},
    vault::SecretField,
};
use serde::Serialize;
use serde_json::{json, Value};
use zeroize::Zeroizing;

use super::{
    browser_input::{key_events, KEY},
    Cdp,
};
use crate::local_tools::{computer::FilledDocuments, ToolError};

/// The isolated world's name, which Chrome shows in its context list.
/// Each fill makes a new world on the document, kept until the document
/// unloads: a long-lived page with many fills holds them all (memory only).
const WORLD: &str = "bruhs-vault";

/// Takes no value. Checks the document's origin, finds the one field,
/// applies the field-kind rule, refuses (`g`) a field whose form declares
/// `method=get`, refuses a submit (`u`) outside a sign-in step, and keeps
/// the field for [`WRITE`]. A sign-in step is a field whose form (or the
/// document, for a field in none) holds a password field, or whose
/// `autocomplete` names one of `a` ([`SIGN_IN_AUTOCOMPLETE`]).
const CHECK: &str = r"function(s,o,t,g,u,a){if(location.origin!==o)return{ok:false,why:'origin'};let es;try{es=document.querySelectorAll(s)}catch(_){return{ok:false,why:'missing'}}if(es.length!==1)return{ok:false,why:'missing'};const e=es[0];if(!(e instanceof HTMLInputElement)||!t.includes(e.type))return{ok:false,why:'kind'};if(e.disabled||e.readOnly)return{ok:false,why:'editable'};if(g&&e.form&&(e.form.getAttribute('method')||'').trim().toLowerCase()==='get')return{ok:false,why:'get'};if(u&&!(e.form||document).querySelector('input[type=password]')&&!(e.getAttribute('autocomplete')||'').toLowerCase().split(/\s+/).some(x=>a.includes(x)))return{ok:false,why:'submit'};globalThis.__bruhsVaultField=e;return{ok:true}}";

/// Checks the origin and the kept field again, focuses it (which runs page
/// code, so the kind is checked once more after), sets the value through
/// the native setter and dispatches `input` and `change`, in one task.
/// With `g`, the field's form never submits natively with GET while it
/// holds a value (a form with no method sends with GET, as does a submit
/// button with `formmethod=get`): the submit is cancelled, so the value
/// never lands in a page address. A page's own script submits as it likes.
const WRITE: &str = r"function(o,t,g,v){const e=globalThis.__bruhsVaultField;globalThis.__bruhsVaultField=null;const fits=()=>location.origin===o&&e instanceof HTMLInputElement&&e.isConnected&&e.ownerDocument===document&&t.includes(e.type)&&!e.disabled&&!e.readOnly;if(!fits())return{ok:false,why:'moved'};e.focus();if(!fits())return{ok:false,why:'moved'};Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(e,v);if(g){document.addEventListener('submit',x=>{const f=x.target,b=x.submitter,m=b&&b.hasAttribute('formmethod')?b.formMethod:f.method;if(m==='get'&&e.form===f&&e.value)x.preventDefault()},true)}e.dispatchEvent(new Event('input',{bubbles:true}));e.dispatchEvent(new Event('change',{bubbles:true}));return{ok:true}}";

fn fill_failed() -> ToolError {
    ToolError::Invalid(FILL_FAILED.into())
}

/// The reply to a fill: which field and whether it submitted, never the
/// value.
pub(in crate::local_tools) fn filled_reply(field: SecretField, submit: bool) -> ToolReply {
    ToolReply {
        value: json!({"filled": field, "submitted": submit}),
        image: None,
    }
}

/// The main frame's id and its document's marks: its loader (first) and its
/// history entry. Back without the back/forward cache loads a new document
/// on the same entry, with form values restored, so both count.
pub(super) async fn document_marks(cdp: &mut Cdp) -> Result<(String, Vec<String>), ToolError> {
    let history = cdp.call("Page.getNavigationHistory", json!({})).await?;
    let tree = cdp.call("Page.getFrameTree", json!({})).await?;
    let frame = &tree["frameTree"]["frame"];
    let entry = history["currentIndex"]
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| history["entries"][index]["id"].as_i64());
    match (frame["id"].as_str(), frame["loaderId"].as_str(), entry) {
        (Some(frame), Some(loader), Some(entry)) => Ok((
            frame.to_owned(),
            vec![format!("loader:{loader}"), format!("entry:{entry}")],
        )),
        _ => Err(fill_failed()),
    }
}

/// Type `value` into the one input `selector` names on the main document,
/// only while that document is on `origin`, then press Enter when
/// `submit`. A submit is refused on a document in `typed`, the ones the
/// bruh typed on itself, by the marks this call reads (rereview M2). Each
/// document it may type into is marked in `filled` before the value is
/// sent; a fill refused before that marks nothing. Every failure reads
/// [`FILL_FAILED`], apart from the page's own answers: another origin, the
/// wrong kind of field, or a submit outside a sign-in step.
#[expect(clippy::too_many_arguments, reason = "one fill's fixed inputs")]
pub(in crate::local_tools) async fn fill_secret(
    cdp: &mut Cdp,
    selector: &str,
    field: SecretField,
    origin: &str,
    value: &str,
    submit: bool,
    typed: &FilledDocuments,
    filled: &mut FilledDocuments,
) -> Result<ToolReply, ToolError> {
    let submit = submit.then_some(typed);
    let checked = check(cdp, selector, field, origin, submit).await?;
    let now = write(cdp, checked, field, origin, value, filled).await?;
    if let Some(typed) = submit {
        // Enter goes to whichever document is loaded now.
        if now.is_none_or(|marks| typed.holds_any(&marks)) {
            return Err(ToolError::Refused(SUBMIT_ONLY_SIGNS_IN.into()));
        }
        let enter = cdp_key("Enter").ok_or_else(fill_failed)?;
        for event in key_events(&enter, 0) {
            cdp.call(KEY, event).await.map_err(|_| fill_failed())?;
        }
    }
    Ok(filled_reply(field, submit.is_some()))
}

/// A field [`CHECK`] found and kept: the isolated world it is kept in,
/// and the marks of the document that world was made on.
pub(super) struct Checked {
    context: i64,
    marks: Vec<String>,
}

/// Creates an isolated world on the main frame's document and runs
/// [`CHECK`] in it, with the submit rule when `submit` holds the documents
/// the bruh typed on, which it refuses to submit on. Types nothing.
pub(super) async fn check(
    cdp: &mut Cdp,
    selector: &str,
    field: SecretField,
    origin: &str,
    submit: Option<&FilledDocuments>,
) -> Result<Checked, ToolError> {
    let typed_on = |marks: &[String]| submit.is_some_and(|typed| typed.holds_any(marks));
    let (frame, marks) = document_marks(cdp).await.map_err(|_| fill_failed())?;
    if typed_on(&marks) {
        return Err(ToolError::Refused(SUBMIT_ONLY_SIGNS_IN.into()));
    }
    let world = cdp
        .call(
            "Page.createIsolatedWorld",
            json!({"frameId": frame, "worldName": WORLD}),
        )
        .await
        .map_err(|_| fill_failed())?;
    let context = world["executionContextId"]
        .as_i64()
        .ok_or_else(fill_failed)?;
    let checked = cdp
        .call(
            "Runtime.callFunctionOn",
            json!({
                "functionDeclaration": CHECK,
                "executionContextId": context,
                "arguments": [
                    {"value": selector},
                    {"value": origin},
                    {"value": secret_input_types(field)},
                    {"value": stays_out_of_addresses(field)},
                    {"value": submit.is_some()},
                    {"value": SIGN_IN_AUTOCOMPLETE},
                ],
                "returnByValue": true,
                "awaitPromise": false,
                "userGesture": false,
            }),
        )
        .await
        .map_err(|_| fill_failed())?;
    answered(&checked)?;
    // The world sits on the document whose marks were read only if that
    // document is still the one loaded: a navigation before the world was
    // made (and another after) would leave the filled one unmarked.
    let (_, again) = document_marks(cdp).await.map_err(|_| fill_failed())?;
    if again.first() != marks.first() {
        return Err(fill_failed());
    }
    // Its history entry may have changed under the same document.
    if typed_on(&again) {
        return Err(ToolError::Refused(SUBMIT_ONLY_SIGNS_IN.into()));
    }
    Ok(Checked { context, marks })
}

/// Marks the checked document, runs [`WRITE`] with the value in its world,
/// then marks the main document again: one that changed while the world
/// was made is marked too. Returns the main document's marks after the
/// write, `None` when Chrome didn't say.
pub(super) async fn write(
    cdp: &mut Cdp,
    Checked { context, marks }: Checked,
    field: SecretField,
    origin: &str,
    value: &str,
    filled: &mut FilledDocuments,
) -> Result<Option<Vec<String>>, ToolError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Params<'a> {
        function_declaration: &'static str,
        execution_context_id: i64,
        arguments: Arguments<'a>,
        return_by_value: bool,
        await_promise: bool,
        user_gesture: bool,
    }
    #[derive(Serialize)]
    struct Argument<T> {
        value: T,
    }
    /// Origin, field types, whether to keep it out of addresses, value.
    type Arguments<'a> = (
        Argument<&'a str>,
        Argument<&'a [&'a str]>,
        Argument<bool>,
        Argument<&'a str>,
    );
    let params = Params {
        function_declaration: WRITE,
        execution_context_id: context,
        arguments: (
            Argument { value: origin },
            Argument {
                value: secret_input_types(field),
            },
            Argument {
                value: stays_out_of_addresses(field),
            },
            Argument { value },
        ),
        return_by_value: true,
        await_promise: false,
        // The page's listeners get no activation to open popups with.
        user_gesture: false,
    };
    // JSON escapes a byte as six at most.
    let room = value
        .len()
        .saturating_add(origin.len())
        .saturating_mul(6)
        .saturating_add(WRITE.len())
        .saturating_add(512);
    // Marked before the value leaves, so a fill cut off after it still
    // counts.
    filled.mark(marks);
    let written = cdp
        .call_secret("Runtime.callFunctionOn", &params, room)
        .await;
    let now = document_marks(cdp).await.ok().map(|(_, marks)| marks);
    if let Some(marks) = &now {
        filled.mark(marks.iter().cloned());
    }
    // Chrome's error, which may quote what it was sent, is dropped here.
    answered(&written.map_err(|_| fill_failed())?)?;
    Ok(now)
}

/// The page's `{ok, why}` answer as a result.
fn answered(result: &Value) -> Result<(), ToolError> {
    if result.get("exceptionDetails").is_some() {
        return Err(fill_failed());
    }
    let answer = &result["result"]["value"];
    if answer["ok"] == true {
        return Ok(());
    }
    Err(match answer["why"].as_str() {
        Some("origin") => ToolError::Refused(LOGIN_WRONG_ORIGIN.into()),
        Some("kind") => ToolError::Invalid(SECRET_FIELD_MISMATCH.into()),
        Some("get") => ToolError::Refused(FILL_GET_FORM.into()),
        Some("submit") => ToolError::Refused(SUBMIT_ONLY_SIGNS_IN.into()),
        _ => fill_failed(),
    })
}

/// A CDP frame for `params` holding a vault value, written into a buffer
/// of `room` bytes that never grows (a grown buffer would leave a copy
/// behind) and is zeroed when dropped.
pub(super) fn secret_frame<P: Serialize>(
    id: u64,
    method: &str,
    params: &P,
    room: usize,
) -> Result<Zeroizing<Vec<u8>>, ToolError> {
    #[derive(Serialize)]
    struct Frame<'a, P> {
        id: u64,
        method: &'a str,
        params: &'a P,
    }
    struct Fixed<'a>(&'a mut Vec<u8>);
    impl std::io::Write for Fixed<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > self.0.capacity() {
                return Err(std::io::ErrorKind::OutOfMemory.into());
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Zeroizing::new(Vec::with_capacity(room));
    let mut writer = Fixed(&mut buffer);
    serde_json::to_writer(&mut writer, &Frame { id, method, params }).map_err(|_| fill_failed())?;
    writer.flush().map_err(|_| fill_failed())?;
    Ok(buffer)
}
