//! Loading a built module and driving it the way the host does.
//!
//! The host runs its plugins on `wasmi` 0.31; so does the harness, only
//! in-process and without the sandbox's frills. A test suite loads the
//! module its own crate builds, calls `plan`, feeds canned answers to
//! `fulfil`, and never touches the network.

use std::{fmt, path::PathBuf, process::Command};

use serde_json::json;
use wasmi::{Engine, Instance, Linker, Memory, Module, Store, TypedFunc};

use crate::ABI_VERSION;

/// One answer of the host to one request of `plan`, in plan order.
#[derive(Clone, Debug)]
pub struct Response {
    /// The HTTP status the host's fetch saw.
    pub status: u16,
    /// The body, as the wire carried it.
    pub body: String,
}

impl From<(u16, &str)> for Response {
    fn from((status, body): (u16, &str)) -> Self {
        Self {
            status,
            body: body.to_string(),
        }
    }
}

/// Everything a harness call can trip over, with the reason attached.
#[derive(Clone, Debug)]
pub struct HarnessError(String);

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for HarnessError {}

impl From<wasmi::Error> for HarnessError {
    fn from(error: wasmi::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<wasmi::core::Trap> for HarnessError {
    fn from(error: wasmi::core::Trap) -> Self {
        Self(error.to_string())
    }
}

impl From<std::io::Error> for HarnessError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<std::string::FromUtf8Error> for HarnessError {
    fn from(error: std::string::FromUtf8Error) -> Self {
        Self(error.to_string())
    }
}

impl From<String> for HarnessError {
    fn from(error: String) -> Self {
        Self(error)
    }
}

/// A loaded plugin: every export the ABI asks for, typed and ready.
pub struct Plugin {
    memory: Memory,
    store: Store<()>,
    alloc_fn: TypedFunc<i32, i32>,
    dealloc_fn: TypedFunc<(i32, i32), ()>,
    abi_version_fn: TypedFunc<(), i32>,
    manifest_fn: TypedFunc<(), i64>,
    plan_fn: TypedFunc<(i32, i32), i64>,
    fulfil_fn: TypedFunc<(i32, i32, i32, i32), i64>,
}

impl Plugin {
    /// Loads a module and checks it speaks this ABI.
    pub fn from_bytes(wasm: &[u8]) -> Result<Self, HarnessError> {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm)?;
        let mut store = Store::new(&engine, ());
        // A v1 module imports nothing, so an empty linker fits it exactly.
        let instance = Linker::<()>::new(&engine)
            .instantiate(&mut store, &module)?
            .start(&mut store)?;
        let mut plugin = Self {
            memory: instance
                .get_memory(&store, "memory")
                .ok_or_else(|| HarnessError("the module exports no `memory`".into()))?,
            alloc_fn: instance.get_typed_func(&store, "alloc")?,
            dealloc_fn: instance.get_typed_func(&store, "dealloc")?,
            abi_version_fn: instance.get_typed_func(&store, "abi_version")?,
            manifest_fn: instance.get_typed_func(&store, "manifest")?,
            plan_fn: instance.get_typed_func(&store, "plan")?,
            fulfil_fn: instance.get_typed_func(&store, "fulfil")?,
            store,
        };
        let reported = plugin.abi_version()?;
        if reported != ABI_VERSION {
            return Err(HarnessError(format!(
                "the module speaks ABI {reported}, this harness speaks {ABI_VERSION}"
            )));
        }
        Ok(plugin)
    }

    /// Loads the module built by the crate `name` (e.g.
    /// `"woofer-plugin-translate"`) — the release wasm of the crate whose
    /// test suite is calling, built on the spot when it is not there yet.
    pub fn from_local_artifact(name: &str) -> Result<Self, HarnessError> {
        Self::from_bytes(&local_artifact(name)?)
    }

    /// The ABI version the module says it speaks.
    pub fn abi_version(&mut self) -> Result<i32, HarnessError> {
        Ok(self.abi_version_fn.call(&mut self.store, ())?)
    }

    /// The manifest, verbatim.
    pub fn manifest(&mut self) -> Result<String, HarnessError> {
        let packed = self.manifest_fn.call(&mut self.store, ())?;
        self.unpack(packed)
    }

    /// One call of `plan`: the call's input in, the plugin's requests out.
    pub fn plan(&mut self, input: &str) -> Result<String, HarnessError> {
        let (ptr, len) = self.give(input.as_bytes())?;
        let packed = self.plan_fn.call(&mut self.store, (ptr, len))?;
        self.unpack(packed)
    }

    /// One call of `fulfil`: the call's input in, the host's answers in
    /// their own buffer alongside, the plugin's output out.
    pub fn fulfil(&mut self, input: &str, responses: &[Response]) -> Result<String, HarnessError> {
        let (input_ptr, input_len) = self.give(input.as_bytes())?;
        let answers = json!({
            "responses": responses
                .iter()
                .map(|answer| json!({ "status": answer.status, "body": answer.body }))
                .collect::<Vec<_>>(),
        });
        let (answers_ptr, answers_len) = self.give(answers.to_string().as_bytes())?;
        let packed = self.fulfil_fn.call(
            &mut self.store,
            (input_ptr, input_len, answers_ptr, answers_len),
        )?;
        self.unpack(packed)
    }

    /// Hands bytes to the plugin: asks for room, then writes.
    fn give(&mut self, bytes: &[u8]) -> Result<(i32, i32), HarnessError> {
        let ptr = self
            .alloc_fn
            .call(&mut self.store, bytes.len() as i32)
            .map_err(|error| HarnessError(format!("the plugin's `alloc` failed: {error}")))?;
        if ptr == 0 {
            return Err(HarnessError(
                "the plugin refused to make room for an argument".into(),
            ));
        }
        self.memory
            .write(&mut self.store, ptr as u32 as usize, bytes)
            .map_err(|error| {
                HarnessError(format!("cannot write the plugin's argument: {error}"))
            })?;
        Ok((ptr, bytes.len() as i32))
    }

    /// Reads the packed answer, frees it, and turns it into a string.
    fn unpack(&mut self, packed: i64) -> Result<String, HarnessError> {
        if packed == 0 {
            return Err(HarnessError(
                "the plugin returned nothing: its memory ran out".into(),
            ));
        }
        let ptr = (packed as u64 >> 32) as u32 as usize;
        let len = (packed as u64 & 0xFFFF_FFFF) as usize;
        if len == 0 {
            return Ok(String::new());
        }
        let mut bytes = vec![0u8; len];
        self.memory
            .read(&self.store, ptr, &mut bytes)
            .map_err(|error| HarnessError(format!("cannot read the plugin's answer: {error}")))?;
        self.dealloc_fn
            .call(&mut self.store, (ptr as i32, len as i32))
            .map_err(|error| HarnessError(format!("the plugin's `dealloc` failed: {error}")))?;
        Ok(String::from_utf8(bytes)?)
    }
}

/// The bytes of the release module the crate `name` builds: the
/// `PLUGIN_WASM` file when the environment names one, else
/// `target/wasm32-unknown-unknown/release/<name>.wasm`, built through cargo
/// when it is not there yet.
fn local_artifact(name: &str) -> Result<Vec<u8>, HarnessError> {
    if let Ok(path) = std::env::var("PLUGIN_WASM") {
        return std::fs::read(path).map_err(HarnessError::from);
    }
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .map_err(|_| HarnessError("CARGO_MANIFEST_DIR is unset: run through cargo".into()))?;
    let module = PathBuf::from(&manifest_dir)
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", name.replace('-', "_")));
    if !module.exists() {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let built = Command::new(cargo)
            .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
            .current_dir(&manifest_dir)
            .status()?;
        if !built.success() {
            return Err(HarnessError(
                "building the module for the harness failed".into(),
            ));
        }
    }
    std::fs::read(&module).map_err(HarnessError::from)
}
