//! The Romanize plugin: Latin-letter spellings for lines written in
//! scripts one cannot sing from, from the endpoint the Google Translate
//! page itself uses — no account, no key.
//!
//! Romanization loses the newlines, so it is asked for one line at a time.
//! A chorus sings its lines again, so each distinct line asks once; lines
//! already written in plain ASCII letters have nothing to spell; and one
//! stubborn line must not cost the whole song its spelling.

use serde::Deserialize;
use serde_json::json;

use woofer_plugin_sdk::register_plugin;

/// Who this plugin is, word for word at the ABI.
const MANIFEST: &str = r#"{
    "id": "romanize",
    "name": "Romanize",
    "publisher": "kreatzzz",
    "version": "1.0.0",
    "api": 1,
    "capabilities": ["provider:romanize"],
    "domains": ["clients5.google.com"],
    "homepage": "https://github.com/kreatzzz/woofer-plugin-romanize"
}"#;

/// The endpoint the Google Translate page itself uses.
const API: &str = "https://clients5.google.com/translate_a/single";

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

fn plan(input: &str) -> Result<String, String> {
    let input = parse(input)?;
    let requests = distinct(&input.lines)
        .iter()
        .map(|line| json!({ "url": query_url(&input.target, line) }))
        .collect::<Vec<_>>();
    Ok(json!({ "requests": requests }).to_string())
}

fn fulfil(input: &str) -> Result<String, String> {
    let input = parse(input)?;
    let wanted = distinct(&input.lines);
    if input.responses.len() != wanted.len() {
        return Err(format!(
            "expected {} answers, got {}",
            wanted.len(),
            input.responses.len()
        ));
    }
    let mut spellings: Vec<(&str, Option<String>)> = Vec::new();
    for (line, answer) in wanted.iter().zip(&input.responses) {
        // One stubborn line must not cost the whole song its spelling.
        spellings.push((line, spelling(line, answer)));
    }
    let romanized = input
        .lines
        .iter()
        .map(|line| {
            spellings
                .iter()
                .find(|(text, _)| *text == line.as_str())
                .and_then(|(_, spelled)| spelled.clone())
        })
        .collect::<Vec<_>>();
    let translated = vec![serde_json::Value::Null; input.lines.len()];
    Ok(json!({
        "romanized": romanized,
        "translated": translated,
    })
    .to_string())
}

/// Reads the call's input, and refuses what is not this plugin's to serve.
fn parse(input: &str) -> Result<Input, String> {
    let input: Input =
        serde_json::from_str(input).map_err(|error| format!("malformed input: {error}"))?;
    if input.kind != "romanize" {
        return Err(format!(
            "romanize serves \"romanize\", not \"{}\"",
            input.kind
        ));
    }
    if input.target.is_empty() {
        return Err("no target language".into());
    }
    Ok(input)
}

/// The lines worth asking for, in the order first sung: not plain ASCII
/// letters, and not a line the song already repeats.
fn distinct<'a>(lines: &'a [String]) -> Vec<&'a str> {
    let mut found: Vec<&'a str> = Vec::new();
    for line in lines {
        if line.is_ascii() || found.contains(&line.as_str()) {
            continue;
        }
        found.push(line.as_str());
    }
    found
}

/// The spelling to keep for a line: `None` when the fetch failed, Google
/// had nothing, or spelled out exactly what was written already.
fn spelling(line: &str, answer: &Answer) -> Option<String> {
    if !(200..300).contains(&answer.status) {
        return None;
    }
    let response: serde_json::Value = serde_json::from_str(&answer.body).ok()?;
    let spelled = response
        .get(0)?
        .as_array()?
        .iter()
        .find_map(|segment| segment.get(3).and_then(|part| part.as_str()))
        .map(str::to_string)?;
    (spelled != line).then_some(spelled)
}

/// One line's spelling. Romanization loses newlines, so it is asked for a
/// single line, never a batch.
fn query_url(target: &str, line: &str) -> String {
    format!(
        "{API}?client=dict-chrome-ex&dt=rm&sl=auto&tl={}&q={}",
        urlencoding::encode(target),
        urlencoding::encode(line),
    )
}
