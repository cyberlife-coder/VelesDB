//! A single generic [`napi::Task`] that runs a boxed blocking closure on the
//! libuv thread pool and resolves its result as a Promise. Every memory
//! operation is CPU/disk-bound (and the Ollama path makes blocking HTTP calls),
//! so all of them go through here off the JS event-loop thread — there are no
//! synchronous variants.

use napi::bindgen_prelude::{ToNapiValue, TypeName};
use napi::{Env, Error, Result, Task};

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
