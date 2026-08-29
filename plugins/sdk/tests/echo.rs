//! The harness, proven on a toy module written by hand: seven exports, no
//! imports, and the packing done in WAT where the SDK cannot fudge it.
//!
//! Runs with the harness feature: `cargo test --features harness`.
#![cfg(feature = "harness")]

use woofer_plugin_sdk::harness::{Plugin, Response};

const MANIFEST: &str = r#"{"id":"echo","name":"Echo","api":1}"#;
/// The toy module bumps allocations from here, so no buffer ever lands on
/// another.
const HEAP: u32 = 2048;

/// An echo plugin in WAT: `plan` and `fulfil` hand their input straight
/// back, manifest and ABI version live at fixed addresses.
fn module(abi_version: i32) -> Vec<u8> {
    let manifest = MANIFEST.replace('\\', "\\\\").replace('"', "\\\"");
    wat::parse_str(format!(
        r#"(module
            (memory (export "memory") 1)
            (data (i32.const 1024) "{manifest}")
            (global $next (mut i32) (i32.const {heap}))
            (func (export "abi_version") (result i32) (i32.const {abi_version}))
            (func (export "alloc") (param $len i32) (result i32)
                (local $ptr i32)
                (local.set $ptr (global.get $next))
                (global.set $next (i32.add (global.get $next) (local.get $len)))
                (local.get $ptr))
            (func (export "dealloc") (param i32 i32))
            (func (export "manifest") (result i64)
                (call $pack (i32.const 1024) (i32.const {manifest_len})))
            (func (export "plan") (param $ptr i32) (param $len i32) (result i64)
                (call $pack (local.get $ptr) (local.get $len)))
            (func (export "fulfil") (param i32 i32 i32 i32) (result i64)
                (call $pack (local.get 0) (local.get 1)))
            (func $pack (param $ptr i32) (param $len i32) (result i64)
                (i64.or
                    (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
                    (i64.extend_i32_u (local.get $len))))
        )"#,
        manifest = manifest,
        manifest_len = MANIFEST.len(),
        heap = HEAP,
        abi_version = abi_version,
    ))
    .unwrap()
}

#[test]
fn the_harness_loads_a_module_and_speaks_its_abi() {
    let mut plugin = Plugin::from_bytes(&module(1)).unwrap();
    assert_eq!(plugin.abi_version().unwrap(), 1);
    assert_eq!(plugin.manifest().unwrap(), MANIFEST);
}

#[test]
fn arguments_and_answers_survive_the_packing() {
    let mut plugin = Plugin::from_bytes(&module(1)).unwrap();
    let input = r#"{"kind":"translate","target":"en","lines":["hello"]}"#;
    assert_eq!(plugin.plan(input).unwrap(), input);
    let answers = [Response::from((200, r#"[["hola"]]"#))];
    assert_eq!(plugin.fulfil(input, &answers).unwrap(), input);
}

#[test]
fn a_module_refusing_to_speak_this_abi_is_refused() {
    let error = Plugin::from_bytes(&module(7)).err().unwrap().to_string();
    assert!(error.contains("7"), "the wrong version is named: {error}");
}
