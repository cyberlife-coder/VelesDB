//! A single generic [`napi::Task`] that runs a boxed blocking closure on the
//! libuv thread pool and resolves its result as a Promise. Every memory
//! operation is CPU/disk-bound (and the Ollama path makes blocking HTTP calls),
//! so all of them go through here off the JS event-loop thread — there are no
//! synchronous variants.

use napi::bindgen_prelude::{ToNapiValue, TypeName, ValueType};
use napi::{Env, Error, Result, Task};

/// Arbitrary JSON resolved as-is to JavaScript (`serde_json::Value` alone
/// cannot be a [`Task`] output: it implements [`ToNapiValue`] but not
/// [`TypeName`]). Keeps the exact wire shape — field names and `null`
/// included — where a `#[napi(object)]` DTO would re-case the keys.
pub struct JsonOut(pub serde_json::Value);

impl TypeName for JsonOut {
    fn type_name() -> &'static str {
        "object"
    }

    fn value_type() -> ValueType {
        ValueType::Object
    }
}

// `ToNapiValue::to_napi_value` is an `unsafe fn` by napi's trait design; this
// impl adds no unsafe operations of its own — it only delegates to the
// existing `serde_json::Value` implementation.
#[allow(unsafe_code)]
impl ToNapiValue for JsonOut {
    unsafe fn to_napi_value(env: napi::sys::napi_env, val: Self) -> Result<napi::sys::napi_value> {
        serde_json::Value::to_napi_value(env, val.0)
    }
}

/// A deferred unit of blocking work producing `O`.
pub struct Job<O: Send + 'static> {
    work: Option<Box<dyn FnOnce() -> Result<O> + Send>>,
}

impl<O: Send + 'static> Job<O> {
    /// Wrap a blocking closure to be run on the libuv pool.
    pub fn new(work: impl FnOnce() -> Result<O> + Send + 'static) -> Self {
        Self {
            work: Some(Box::new(work)),
        }
    }
}

impl<O: Send + 'static + ToNapiValue + TypeName> Task for Job<O> {
    type Output = O;
    type JsValue = O;

    fn compute(&mut self) -> Result<Self::Output> {
        let work = self
            .work
            .take()
            .ok_or_else(|| Error::from_reason("[INTERNAL] task computed twice"))?;
        work()
    }

    fn resolve(&mut self, _env: Env, output: Self::Output) -> Result<Self::JsValue> {
        Ok(output)
    }
}
