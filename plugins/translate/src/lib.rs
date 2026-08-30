//! The Translate plugin: the reader's language under every line, from the
//! endpoint the Google Translate page itself uses — no account, no key.
//!
//! Translation batches well, so the lines ride together: joined with
//! newlines, split into chunks whose query fits a URL, and put back on
//! their lines when the answers come home. Romanization loses the
//! newlines, so it is another plugin's work; this one never asks for it.

use serde::Deserialize;
use serde_json::json;

use woofer_plugin_sdk::register_plugin;

/// Who this plugin is, word for word at the ABI.
const MANIFEST: &str = r#"{
    "id": "translate",
    "name": "Translate",
    "publisher": "kreatzzz",
    "version": "1.0.0",
    "api": 1,
    "capabilities": ["provider:translate"],
    "domains": ["clients5.google.com"],
    "homepage": "https://github.com/kreatzzz/woofer-plugin-translate"
}"#;

/// The endpoint the Google Translate page itself uses.
const API: &str = "https://clients5.google.com/translate_a/single";
/// The room one request's query may take, counted so carefully that no
/// song, however dense with wide letters, pushes the URL past what servers
/// accept.
const MAX_QUERY: usize = 6000;

register_plugin! {
    manifest = MANIFEST,
    plan = plan,
    fulfil = fulfil,
}

/// One call's input: at plan time without `responses`, at fulfil time with.
#[derive(Deserialize)]
struct Input {
    kind: String,
    target: String,
    lines: Vec<String>,
    #[serde(default)]
    responses: Vec<Answer>,
}

/// One answer of the host to one request of `plan`, in plan order.
#[derive(Deserialize)]
struct Answer {
    #[serde(default)]
    status: u16,
    #[serde(default)]
    body: String,
}

impl Answer {
    /// The answer's JSON, when the fetch itself worked.
    fn json(&self) -> Result<serde_json::Value, String> {
        if !(200..300).contains(&self.status) {
            return Err(format!("Google Translate answered {}", self.status));
        }
        serde_json::from_str(&self.body)
            .map_err(|error| format!("unexpected answer from Google Translate: {error}"))
    }
}

fn plan(input: &str) -> Result<String, String> {
    let input = parse(input)?;
    // Nothing to say: an empty song asks for nothing.
    if lines_all_empty(&input.lines) {
        return Ok(json!({ "requests": [] }).to_string());
    }
    let requests = chunk_lines(&input.lines)
        .iter()
        .map(|chunk| json!({ "url": query_url(&input.target, chunk) }))
        .collect::<Vec<_>>();
    Ok(json!({ "requests": requests }).to_string())
}

fn fulfil(input: &str) -> Result<String, String> {
    let input = parse(input)?;
    let count = input.lines.len();
    if lines_all_empty(&input.lines) {
        return Ok(empty(count));
    }
    let chunks = chunk_lines(&input.lines);
    if input.responses.len() != chunks.len() {
        return Err(format!(
            "expected {} answers, got {}",
            chunks.len(),
            input.responses.len()
        ));
    }
    // The first answer tells the tongue the song was heard in; when that
    // is the reader's own, there is nothing to add.
    let first = input.responses[0].json()?;
    let source = first
        .get(2)
        .and_then(|part| part.as_str())
        .ok_or("unexpected answer from Google Translate")?;
    if same_language(source, &input.target) {
        return Ok(empty(count));
    }
    // An answer keeps the newlines inside a chunk but not the one at its
    // end, so the chunks are stitched back with the newline each lost.
    let mut text = String::new();
    for (index, answer) in input.responses.iter().enumerate() {
        if index > 0 {
            text.push('\n');
        }
        text.push_str(&segment_text(&answer.json()?)?);
    }
    let romanized: Vec<Option<String>> = vec![None; count];
    Ok(json!({
        "translated": aligned(&text, count),
        "romanized": romanized,
    })
    .to_string())
}

/// Reads the call's input, and refuses what is not this plugin's to serve.
fn parse(input: &str) -> Result<Input, String> {
    let input: Input =
        serde_json::from_str(input).map_err(|error| format!("malformed input: {error}"))?;
    if input.kind != "translate" {
        return Err(format!(
            "translate serves \"translate\", not \"{}\"",
            input.kind
        ));
    }
    if input.target.is_empty() {
        return Err("no target language".into());
    }
    Ok(input)
}

/// Nothing to say when every line is blank.
fn lines_all_empty(lines: &[String]) -> bool {
    lines.iter().all(|line| line.trim().is_empty())
}

/// Nothing to add anywhere: nulls aligned to the lines.
fn empty(count: usize) -> String {
    json!({
        "translated": vec![serde_json::Value::Null; count],
        "romanized": vec![serde_json::Value::Null; count],
    })
    .to_string()
}

/// One call to the endpoint: the target tongue, then the text — which may
/// hold several lines joined with newlines.
fn query_url(target: &str, text: &str) -> String {
    format!(
        "{API}?client=dict-chrome-ex&dt=t&sl=auto&tl={}&q={}",
        urlencoding::encode(target),
        urlencoding::encode(text),
    )
}

/// The translated text of an answer — every segment's first words, joined.
fn segment_text(response: &serde_json::Value) -> Result<String, String> {
    let segments = response
        .get(0)
        .and_then(|part| part.as_array())
        .ok_or("unexpected answer from Google Translate")?;
    let mut text = String::new();
    for segment in segments {
        // A romanization segment opens with null and says nothing here.
        if let Some(piece) = segment.get(0).and_then(|part| part.as_str()) {
            text.push_str(piece);
        }
    }
    Ok(text)
}

/// Puts a batched answer back on its lines: a line that came back empty
/// keeps `None`, and a trailing line Google dropped stays gone, so the
/// count still matches the song.
fn aligned(text: &str, count: usize) -> Vec<Option<String>> {
    let mut lines: Vec<Option<String>> = text
        .split('\n')
        .map(|line| (!line.is_empty()).then(|| line.to_string()))
        .collect();
    lines.resize(count, None);
    lines
}

/// Whether `source` and `target` name the same tongue, ignoring the region
/// Google appends ("zh-CN") and the case it uses.
fn same_language(source: &str, target: &str) -> bool {
    fn base(code: &str) -> &str {
        code.split_once('-').map_or(code, |(root, _)| root)
    }
    base(source).eq_ignore_ascii_case(base(target))
}

/// The room a piece of text takes in the query string, counted the careful
/// way: an ASCII letter may grow to three (`%20`), any other letter to nine
/// (`%E3%81%82`), so a batch that fits this count fits whatever the encoder
/// really does.
fn encoded_len(text: &str) -> usize {
    text.chars().map(|c| if c.is_ascii() { 3 } else { 9 }).sum()
}

/// Splits `lines` into batches whose query stays inside the budget. Lines
/// keep their order and their newlines, so the answers can be realigned
/// afterwards; a single line longer than the budget gets a request of its
/// own, for no lyric line is that long.
fn chunk_lines(lines: &[String]) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut budget = 0;
    let mut held = 0;
    for line in lines {
        let cost = encoded_len(line) + if held == 0 { 0 } else { 3 };
        if held > 0 && budget + cost > MAX_QUERY {
            chunks.push(std::mem::take(&mut current));
            budget = 0;
            held = 0;
        }
        if held > 0 {
            current.push('\n');
            budget += 3;
        }
        current.push_str(line);
        budget += encoded_len(line);
        held += 1;
    }
    if held > 0 {
        chunks.push(current);
    }
    chunks
}
