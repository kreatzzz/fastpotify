//! The test suite runs the built module the way the host will: loaded into
//! wasmi, driven over the ABI, offline, with canned answers.

use serde_json::{json, Value};
use woofer_plugin_sdk::harness::{Plugin, Response};

fn plugin() -> Plugin {
    Plugin::from_local_artifact("woofer-plugin-translate").expect("built plugin module")
}

fn plan_of(plugin: &mut Plugin, lines: Value, target: &str) -> Value {
    let input = json!({ "kind": "translate", "target": target, "lines": lines });
    serde_json::from_str(&plugin.plan(&input.to_string()).unwrap()).unwrap()
}

fn fulfil_of(plugin: &mut Plugin, lines: Value, target: &str, responses: &[Response]) -> Value {
    let input = json!({ "kind": "translate", "target": target, "lines": lines });
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
    assert_eq!(manifest["id"], "translate");
    assert_eq!(manifest["name"], "Translate");
    assert_eq!(manifest["publisher"], "kreatzzz");
    assert_eq!(manifest["version"], "1.0.0");
    assert_eq!(manifest["api"], 1);
    assert_eq!(manifest["capabilities"], json!(["provider:translate"]));
    assert_eq!(manifest["domains"], json!(["clients5.google.com"]));
    assert!(manifest["homepage"].is_string());
}

#[test]
fn one_request_carries_the_whole_small_song() {
    let mut plugin = plugin();
    let planned = plan_of(&mut plugin, json!(["hello", "world"]), "en");
    let requests = planned["requests"].as_array().unwrap();
    assert_eq!(requests.len(), 1);
    let url = requests[0]["url"].as_str().unwrap();
    assert!(url.starts_with("https://clients5.google.com/translate_a/single?client=dict-chrome-ex"));
    assert!(url.contains("dt=t"), "translation is asked for");
    assert!(
        !url.contains("dt=rm"),
        "romanization is another plugin's work"
    );
    assert!(url.contains("sl=auto"));
    assert!(url.contains("tl=en"));
    assert!(url.contains(&format!("q={}", urlencoding::encode("hello\nworld"))));
}

#[test]
fn an_empty_song_asks_for_nothing_and_keeps_its_nulls() {
    let mut plugin = plugin();
    let planned = plan_of(&mut plugin, json!(["", "  ", ""]), "en");
    assert_eq!(planned["requests"].as_array().unwrap().len(), 0);
    let fulfilled = fulfil_of(&mut plugin, json!(["", "  ", ""]), "en", &[]);
    assert_eq!(fulfilled["translated"], json!([null, null, null]));
    assert_eq!(fulfilled["romanized"], json!([null, null, null]));
}

#[test]
fn a_dense_song_splits_into_url_budgeted_chunks_that_round_trip() {
    let mut plugin = plugin();
    let line = "あ".repeat(100);
    let lines: Vec<String> = (0..20).map(|_| line.clone()).collect();
    let planned = plan_of(&mut plugin, json!(lines), "en");
    let requests = planned["requests"].as_array().unwrap();
    assert!(requests.len() > 1, "a song of CJK lines must split");
    let mut joined = String::new();
    for request in requests {
        let url = request["url"].as_str().unwrap();
        let query = url.rsplit_once("q=").unwrap().1;
        assert!(query.len() <= 6000, "every chunk fits the URL budget");
        if !joined.is_empty() {
            joined.push('\n');
        }
        joined.push_str(&urlencoding::decode(query).unwrap());
    }
    assert_eq!(
        joined,
        lines.join("\n"),
        "the chunks keep every line, in order"
    );
}

#[test]
fn stitched_chunk_answers_line_back_up() {
    let mut plugin = plugin();
    let line = "あ".repeat(100);
    let lines: Vec<String> = (0..20).map(|index| format!("{line}{index}")).collect();
    let planned = plan_of(&mut plugin, json!(lines), "en");
    let mut responses = Vec::new();
    for request in planned["requests"].as_array().unwrap() {
        let chunk = urlencoding::decode(
            request["url"]
                .as_str()
                .unwrap()
                .rsplit_once("q=")
                .unwrap()
                .1,
        )
        .unwrap()
        .into_owned();
        // The canned answer keeps the chunk's newlines, as Google's do.
        responses.push(Response {
            status: 200,
            body: json!([[[chunk, "", null, null, 10]], null, "ja"]).to_string(),
        });
    }
    let fulfilled = fulfil_of(&mut plugin, json!(lines), "en", &responses);
    let translated = fulfilled["translated"].as_array().unwrap();
    assert_eq!(translated.len(), 20);
    for (index, piece) in translated.iter().enumerate() {
        let piece = piece.as_str().unwrap();
        assert!(piece.starts_with("あ"));
        assert!(piece.ends_with(&index.to_string()));
    }
    assert_eq!(fulfilled["romanized"], json!(vec![Value::Null; 20]));
}

#[test]
fn answers_land_back_on_their_lines() {
    let mut plugin = plugin();
    // Google answers a batch in pieces, newlines inside the pieces. The
    // answer's first element is the segments, each carrying its words.
    let answer = json!([
        [["hola\nmundo\n\nbien", "hello", null, null, 10]],
        null,
        "es"
    ])
    .to_string();
    let fulfilled = fulfil_of(
        &mut plugin,
        json!(["hello", "world", "", "fine"]),
        "en",
        &[Response {
            status: 200,
            body: answer,
        }],
    );
    assert_eq!(
        fulfilled["translated"],
        json!(["hola", "mundo", null, "bien"])
    );
    // Romanization is another plugin's work.
    assert_eq!(fulfilled["romanized"], json!([null, null, null, null]));
}

#[test]
fn a_line_google_dropped_stays_null() {
    let mut plugin = plugin();
    let answer = json!([[["one\ntwo", "", null, null, 10]], null, "es"]).to_string();
    let fulfilled = fulfil_of(
        &mut plugin,
        json!(["one", "two", "three"]),
        "en",
        &[Response {
            status: 200,
            body: answer,
        }],
    );
    assert_eq!(fulfilled["translated"], json!(["one", "two", null]));

    // And an answer with a line too many is cut back to the count.
    let answer = json!([[["one\ntwo\nthree", "", null, null, 10]], null, "es"]).to_string();
    let fulfilled = fulfil_of(
        &mut plugin,
        json!(["one", "two"]),
        "en",
        &[Response {
            status: 200,
            body: answer,
        }],
    );
    assert_eq!(fulfilled["translated"], json!(["one", "two"]));
}

#[test]
fn a_song_already_in_the_readers_language_adds_nothing() {
    let mut plugin = plugin();
    let lines = json!(["hello", "world"]);
    let answer = json!([[["hello", "hello", null, null, 10]], null, "en"]).to_string();
    let fulfilled = fulfil_of(
        &mut plugin,
        lines.clone(),
        "en",
        &[Response {
            status: 200,
            body: answer,
        }],
    );
    assert_eq!(fulfilled["translated"], json!([null, null]));
    assert_eq!(fulfilled["romanized"], json!([null, null]));

    // Region suffixes and case do not change the tongue.
    let answer = json!([[["你好", "你好", null, null, 10]], null, "zh-CN"]).to_string();
    let fulfilled = fulfil_of(
        &mut plugin,
        lines,
        "zh-TW",
        &[Response {
            status: 200,
            body: answer,
        }],
    );
    assert_eq!(fulfilled["translated"], json!([null, null]));
}

#[test]
fn a_refused_or_unexpected_answer_is_an_error() {
    let mut plugin = plugin();
    let input = json!({ "kind": "translate", "target": "en", "lines": ["hello"] });
    let refused = plugin
        .fulfil(&input.to_string(), &[Response::from((429, "too many"))])
        .unwrap();
    let error: Value = serde_json::from_str(&refused).unwrap();
    assert!(error["error"].as_str().unwrap().contains("429"));

    let nonsense = Response {
        status: 200,
        body: "[]".into(),
    };
    let refused = plugin.fulfil(&input.to_string(), &[nonsense]).unwrap();
    let error: Value = serde_json::from_str(&refused).unwrap();
    assert!(error["error"]
        .as_str()
        .unwrap()
        .contains("Google Translate"));

    // And answers that do not match the chunks are refused outright.
    let refused = plugin.fulfil(&input.to_string(), &[]).unwrap();
    let error: Value = serde_json::from_str(&refused).unwrap();
    assert!(error["error"].as_str().unwrap().contains("expected 1"));
}

#[test]
fn a_call_meant_for_another_plugin_is_refused() {
    let mut plugin = plugin();
    let input = json!({ "kind": "romanize", "target": "en", "lines": ["こんにちは"] });
    let refused = plugin.plan(&input.to_string()).unwrap();
    let error: Value = serde_json::from_str(&refused).unwrap();
    assert!(error["error"].as_str().unwrap().contains("romanize"));
}
