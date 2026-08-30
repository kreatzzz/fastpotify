//! The ABI, written once so no plugin writes it twice.

use std::alloc::{alloc as global_alloc, dealloc as global_dealloc, Layout};

use serde_json::json;

/// Every allocation's alignment. `dealloc` rebuilds the layout from the
/// length alone, so `alloc` and `dealloc` must never disagree about it.
const ALIGN: usize = 16;

/// The pointer handed back for a zero-length request: aligned, never read.
const ZERO_PTR: i32 = ALIGN as i32;

/// Room for `len` bytes the host may write into: `0` when the request
/// cannot be served, the never-read sentinel [`ZERO_PTR`] when `len` is
/// zero.
pub fn alloc(len: i32) -> i32 {
    if len <= 0 {
        return ZERO_PTR;
    }
    match Layout::from_size_align(len as usize, ALIGN) {
        Ok(layout) => (unsafe { global_alloc(layout) }) as i32,
        Err(_) => 0,
    }
}

/// Frees what [`alloc`] handed out, with the same `len` it was asked for.
pub fn dealloc(ptr: i32, len: i32) {
    if ptr <= 0 || len <= 0 {
        return;
    }
    if let Ok(layout) = Layout::from_size_align(len as usize, ALIGN) {
        unsafe { global_dealloc(ptr as *mut u8, layout) }
    }
}

/// Packs a pointer+len pair the way the ABI carries strings.
fn pack(ptr: i32, len: i32) -> i64 {
    ((((ptr as u32) as u64) << 32) | ((len as u32) as u64)) as i64
}

/// Copies `text` into fresh room and packs it for the host. A packed `0`
/// means the room could not be had; the host reads that as the call
/// failing.
pub fn return_str(text: &str) -> i64 {
    let ptr = alloc(text.len() as i32);
    if ptr == 0 {
        return 0;
    }
    if !text.is_empty() {
        // SAFETY: `alloc` just made room for exactly these bytes.
        unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), ptr as *mut u8, text.len()) };
    }
    pack(ptr, text.len() as i32)
}

/// Takes an argument buffer back: copies what the host wrote, frees the
/// room at once, and hands the bytes on.
fn take_buffer(ptr: i32, len: i32) -> Vec<u8> {
    if ptr <= 0 || len <= 0 {
        return Vec::new();
    }
    // SAFETY: the room came from `alloc` for exactly this length, and the
    // host has only written into it.
    let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, len as usize) }.to_vec();
    dealloc(ptr, len);
    bytes
}

fn take_string(ptr: i32, len: i32) -> String {
    String::from_utf8_lossy(&take_buffer(ptr, len)).into_owned()
}

/// Whatever a handler came back with becomes one packed string: an answer
/// as itself, a refusal as `{"error":…}`.
fn respond(answer: Result<String, String>) -> i64 {
    return_str(&answer.unwrap_or_else(|error| json!({ "error": error }).to_string()))
}

/// Runs `plan`: one argument buffer in, one packed answer out.
pub fn call<F>(plan: F, input_ptr: i32, input_len: i32) -> i64
where
    F: FnOnce(&str) -> Result<String, String>,
{
    let input = take_string(input_ptr, input_len);
    respond(plan(&input))
}

/// Runs `fulfil`: the call's input and the host's answers arrive in two
/// buffers, and the handler sees them as one JSON object with `responses`
/// attached.
pub fn call_with_responses<F>(
    fulfil: F,
    input_ptr: i32,
    input_len: i32,
    responses_ptr: i32,
    responses_len: i32,
) -> i64
where
    F: FnOnce(&str) -> Result<String, String>,
{
    let input = take_string(input_ptr, input_len);
    let responses = take_string(responses_ptr, responses_len);
    respond(attach_responses(input, responses).and_then(|input| fulfil(&input)))
}

/// Folds the second buffer into the first, so the handler sees the call's
/// input with `"responses"` set. An empty responses buffer changes
/// nothing — the host may fold the answers in itself.
fn attach_responses(input: String, responses: String) -> Result<String, String> {
    if responses.trim().is_empty() {
        return Ok(input);
    }
    let answers: serde_json::Value = serde_json::from_str(&responses)
        .map_err(|error| format!("malformed responses: {error}"))?;
    let answers = match answers {
        serde_json::Value::Array(_) => answers,
        serde_json::Value::Object(fields) => fields
            .get("responses")
            .cloned()
            .ok_or_else(|| "malformed responses: no `responses` field".to_string())?,
        _ => return Err("malformed responses: neither object nor array".into()),
    };
    let mut input: serde_json::Value =
        serde_json::from_str(&input).map_err(|error| format!("malformed input: {error}"))?;
    let fields = input
        .as_object_mut()
        .ok_or_else(|| "malformed input: not a JSON object".to_string())?;
    fields.insert("responses".to_string(), answers);
    Ok(input.to_string())
}
