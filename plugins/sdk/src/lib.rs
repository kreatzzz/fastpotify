//! The Woofer plugin SDK.
//!
//! A plugin is one `wasm32-unknown-unknown` module that computes and asks;
//! the host fetches, retries, caches, and decides. The whole conversation
//! runs over ABI version 1: no imports, JSON everywhere, and strings
//! packed into one `i64` as `(ptr as u64) << 32 | len as u64`. A module
//! exports exactly seven things — `memory`, `alloc`, `dealloc`,
//! `abi_version`, `manifest`, `plan`, and `fulfil` — and this crate hides
//! all of them behind one macro. A plugin declares its manifest and two
//! handlers, and the exports appear:
//!
//! ```ignore
//! const MANIFEST: &str = r#"{ "id": "translate", … }"#;
//!
//! /// Decides WHAT to fetch.
//! fn plan(input: &str) -> Result<String, String> { … }
//!
//! /// Parses what came back into output.
//! fn fulfil(input: &str) -> Result<String, String> { … }
//!
//! woofer_plugin_sdk::register_plugin! {
//!     manifest = MANIFEST,
//!     plan = plan,
//!     fulfil = fulfil,
//! }
//! ```
//!
//! `plan` receives the call's input (`{"kind":…,"target":…,"lines":[…]}`) and
//! answers `{"requests":[{"url":…}]}`. `fulfil` receives the same input with
//! the host's answers attached — `"responses":[{"status":200,"body":"…"}]`,
//! in plan order, whether the host hands them over in the second buffer or
//! folds them into the input itself — and answers either `{"error":…}` or
//! the capability's own output shape. A handler returning `Err` lands at
//! the ABI as `{"error":…}`, so the host has one failure shape, not two.
//!
//! # Memory
//!
//! The module runs on the standard global allocator, and `alloc` /
//! `dealloc` are thin wrappers over it. `alloc(len)` returns
//! 16-byte-aligned room for `len` bytes; the aligned sentinel `16` when
//! `len` is zero, a pointer never to read; and `0` when the request cannot
//! be served, which the host reads as out of memory. `dealloc(ptr, len)`
//! must see the same `len` the room was asked for. The glue frees the
//! host's argument buffers as soon as it has copied them; the host frees
//! the plugin's answers through `dealloc` once it has read them.
//!
//! # The harness
//!
//! With the `harness` feature — host side only, since it drags in `wasmi`
//! 0.31, the same interpreter the host runs — a built module loads
//! in-process and is driven exactly the way the host drives it. The
//! plugins' test suites are built on it; see [`harness`].

/// The ABI this SDK generates modules for, and the harness speaks.
pub const ABI_VERSION: i32 = 1;

/// The ABI, written once so no plugin writes it twice. Hidden from the
/// docs: authors meet it through [`register_plugin!`], not by hand.
#[doc(hidden)]
pub mod abi;
#[cfg(feature = "harness")]
pub mod harness;

/// Wires a plugin up: embeds the manifest and generates the ABI's exports,
/// routing `plan` and `fulfil` through the two handlers.
///
/// The handlers take the call's JSON as `&str` and answer JSON — or an
/// `Err(String)`, which becomes `{"error":…}`. The manifest must be a
/// `&'static str` of JSON naming `id`, `name`, `publisher`, `version`,
/// `api`, `capabilities`, `domains`, and `homepage`.
///
/// The generated functions are named `woofer_alloc`, `woofer_plan`, … in
/// Rust, so they cannot collide with the handlers, and carry their ABI
/// names — `alloc`, `plan`, … — through `#[export_name]`.
#[macro_export]
macro_rules! register_plugin {
    (manifest = $manifest:expr, plan = $plan:expr, fulfil = $fulfil:expr $(,)?) => {
        /// Room for the host to write its arguments into.
        #[unsafe(export_name = "alloc")]
        pub extern "C" fn woofer_alloc(len: i32) -> i32 {
            $crate::abi::alloc(len)
        }

        /// Frees what [`woofer_alloc`] handed out.
        #[unsafe(export_name = "dealloc")]
        pub extern "C" fn woofer_dealloc(ptr: i32, len: i32) {
            $crate::abi::dealloc(ptr, len)
        }

        /// The ABI the module speaks.
        #[unsafe(export_name = "abi_version")]
        pub extern "C" fn woofer_abi_version() -> i32 {
            $crate::ABI_VERSION
        }

        /// Who this plugin is, word for word.
        #[unsafe(export_name = "manifest")]
        pub extern "C" fn woofer_manifest() -> i64 {
            $crate::abi::return_str($manifest)
        }

        /// Decides what to fetch, from the call's input alone.
        #[unsafe(export_name = "plan")]
        pub extern "C" fn woofer_plan(input_ptr: i32, input_len: i32) -> i64 {
            $crate::abi::call($plan, input_ptr, input_len)
        }

        /// Parses the host's answers into the capability's output.
        #[unsafe(export_name = "fulfil")]
        pub extern "C" fn woofer_fulfil(
            input_ptr: i32,
            input_len: i32,
            responses_ptr: i32,
            responses_len: i32,
        ) -> i64 {
            $crate::abi::call_with_responses(
                $fulfil,
                input_ptr,
                input_len,
                responses_ptr,
                responses_len,
            )
        }
    };
}
