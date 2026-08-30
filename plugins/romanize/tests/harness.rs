//! The test suite runs the built module the way the host will: loaded into
//! wasmi, driven over the ABI, offline, with canned answers.

use serde_json::{json, Value};
use woofer_plugin_sdk::harness::{Plugin, Response};

fn plugin() -> Plugin {
    Plugin::from_local_artifact("woofer-plugin-romanize").expect("built plugin module")
}

fn plan_of(plugin: &mut Plugin, lines: Value, target: &str) -> Value {
    let input = json!({ "kind": "romanize", "target": target, "lines": lines });
    serde_json::from_str(&plugin.plan(&input.to_string()).unwrap()).unwrap()
}

fn fulfil_of(plugin: &mut Plugin, lines: Value, target: &str, responses: &[Response]) -> Value {
    let input = json!({ "kind": "romanize", "target": target, "lines": lines });
    serde_json::from_str(&plugin.fulfil(&input.to_string(), responses).unwrap()).unwrap()
}

#[test]
fn the_module_speaks_abi_one() {
    let mut plugin = plugin();
    assert_eq!(plugin.abi_version().unwrap(), 1);
}

#[test]
fn the_manifest_declares_what_the_host_needs() {
    let mut plugin = plugin();
    let manifest: Value = serde_json::from_str(&plugin.manifest().unwrap()).unwrap();
    assert_eq!(manifest["id"], "romanize");
    assert_eq!(manifest["name"], "Romanize");
    assert_eq!(manifest["publisher"], "kreatzzz");
    assert_eq!(manifest["version"], "1.0.0");
    assert_eq!(manifest["api"], 1);
    assert_eq!(manifest["capabilities"], json!(["provider:romanize"]));
    assert_eq!(manifest["domains"], json!(["clients5.google.com"]));
    assert!(manifest["homepage"].is_string());
}

#[test]
fn ascii_empty_and_repeated_lines_ask_nothing_twice() {
    let mut plugin = plugin();
    let lines = json!(["こんにちは", "hello", "", "世界", "こんにちは", "世界"]);
    let planned = plan_of(&mut plugin, lines, "en");
    let requests = planned["requests"].as_array().unwrap();
    assert_eq!(requests.len(), 2, "each distinct non-ASCII line asks once");
    for request in requests {
        let url = request["url"].as_str().unwrap();
        assert!(
            url.starts_with("https://clients5.google.com/translate_a/single?client=dict-chrome-ex")
        );
        assert!(url.contains("dt=rm"), "only the spelling is asked for");
        assert!(
            !url.contains("dt=t"),
            "translation is another plugin's work"
        );
        assert!(url.contains("sl=auto"));
        assert!(url.contains("tl=en"));
    }
    let first = requests[0]["url"]
        .as_str()
        .unwrap()
        .rsplit_once("q=")
        .unwrap()
        .1;
    assert_eq!(urlencoding::decode(first).unwrap(), "こんにちは");
    let second = requests[1]["url"]
        .as_str()
        .unwrap()
        .rsplit_once("q=")
        .unwrap()
        .1;
    assert_eq!(urlencoding::decode(second).unwrap(), "世界");
}

#[test]
fn a_song_already_in_latin_letters_asks_for_nothing() {
    let mut plugin = plugin();
    let lines = json!(["hello", "world", ""]);
    let planned = plan_of(&mut plugin, lines.clone(), "en");
    assert_eq!(planned["requests"].as_array().unwrap().len(), 0);
    let fulfilled = fulfil_of(&mut plugin, lines, "en", &[]);
    assert_eq!(fulfilled["romanized"], json!([null, null, null]));
    assert_eq!(fulfilled["translated"], json!([null, null, null]));
}

#[test]
fn answers_map_back_to_every_line() {
    let mut plugin = plugin();
    let lines = json!(["こんにちは", "世界", "こんにちは", "hello"]);
    let planned = plan_of(&mut plugin, lines.clone(), "en");
    assert_eq!(planned["requests"].as_array().unwrap().len(), 2);
    let fulfilled = fulfil_of(
        &mut plugin,
        lines,
        "en",
        &[
            Response {
                status: 200,
                // The answer's first element is the segments, each carrying
                // its spelling at index 3.
                body: json!([[["こんにちは", null, null, "konnichiwa"]]]).to_string(),
            },
            Response {
                status: 200,
                body: json!([[["世界", null, null, "sekai"]]]).to_string(),
            },
        ],
    );
    assert_eq!(
        fulfilled["romanized"],
        json!(["konnichiwa", "sekai", "konnichiwa", null])
    );
    // Translation is another plugin's work.
    assert_eq!(fulfilled["translated"], json!([null, null, null, null]));
}

#[test]
fn a_spelling_that_repeats_the_line_is_no_help() {
    let mut plugin = plugin();
    let lines = json!(["naïve", "こんにちは"]);
    let fulfilled = fulfil_of(
        &mut plugin,
        lines,
        "en",
        &[
            Response {
                status: 200,
                body: json!([[["naïve", null, null, "naïve"]]]).to_string(),
            },
            Response {
                status: 200,
                body: json!([[["こんにちは", null, null, "konnichiwa"]]]).to_string(),
            },
        ],
    );
    assert_eq!(fulfilled["romanized"], json!([null, "konnichiwa"]));
}

#[test]
fn a_failing_line_costs_only_itself() {
    let mut plugin = plugin();
    let lines = json!(["こんにちは", "世界", "안녕하세요"]);
    let fulfilled = fulfil_of(
        &mut plugin,
        lines,
        "en",
        &[
            // A refusal.
            Response {
                status: 500,
                body: String::new(),
            },
            // A body no parser can love.
            Response {
                status: 200,
                body: "not json".into(),
            },
            // An answer that carries no spelling at all.
            Response {
                status: 200,
                body: json!([[["안녕하세요", null, null]]]).to_string(),
            },
        ],
    );
    assert_eq!(fulfilled["romanized"], json!([null, null, null]));
    assert!(fulfilled.get("error").is_none(), "no line fails the song");
}

#[test]
fn answers_that_do_not_match_the_requests_are_an_error() {
    let mut plugin = plugin();
    let input = json!({ "kind": "romanize", "target": "en", "lines": ["こんにちは"] });
    let refused = plugin.fulfil(&input.to_string(), &[]).unwrap();
    let error: Value = serde_json::from_str(&refused).unwrap();
    assert!(error["error"].as_str().unwrap().contains("expected 1"));
}

#[test]
fn a_call_meant_for_another_plugin_is_refused() {
    let mut plugin = plugin();
    let input = json!({ "kind": "translate", "target": "en", "lines": ["hello"] });
    let refused = plugin.plan(&input.to_string()).unwrap();
    let error: Value = serde_json::from_str(&refused).unwrap();
    assert!(error["error"].as_str().unwrap().contains("translate"));
}
