//! napi-rs build setup: emits the platform linker flags the cdylib needs to
//! resolve Node-API symbols at load time.

fn main() {
    napi_build::setup();
}
