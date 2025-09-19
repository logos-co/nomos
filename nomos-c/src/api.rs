use std::ffi::c_void;

use nomos_node::RuntimeServiceId;
use overwatch::overwatch::{Overwatch, OverwatchHandle};
use tokio::runtime::{Handle, Runtime};

use crate::NomosNodeErrorCode;

// Define an opaque type for the complex Overwatch type
type NomosOverwatch = Overwatch<RuntimeServiceId>;

#[repr(C)]
pub struct NomosNode {
    // Use opaque pointer instead of the generic type
    overwatch: *mut c_void,
    // Keep simple types as-is
    runtime: *mut c_void,
}

impl NomosNode {
    pub fn new(overwatch: NomosOverwatch, runtime: Runtime) -> Self {
        Self {
            // Box the complex types and convert to opaque pointers
            overwatch: Box::into_raw(Box::new(overwatch)).cast::<c_void>(),
            runtime: Box::into_raw(Box::new(runtime)).cast::<c_void>(),
        }
    }

    // Helper methods to safely access the inner types
    #[must_use]
    const fn get_overwatch_handle(&self) -> &OverwatchHandle<RuntimeServiceId> {
        unsafe {
            self.overwatch
                .cast::<NomosOverwatch>()
                .as_ref()
                .expect("A valid `NomosOverwatch not null pointer`")
        }
        .handle()
    }

    #[must_use]
    fn get_runtime_handle(&self) -> &Handle {
        unsafe {
            self.runtime
                .cast::<Runtime>()
                .as_ref()
                .expect("A valid `tokio::Runtime` not null pointer")
        }
        .handle()
    }

    // Helper to safely take ownership back
    #[must_use]
    pub fn into_parts(self) -> (Box<NomosOverwatch>, Box<Runtime>) {
        let overwatch = unsafe { Box::from_raw(self.overwatch.cast::<NomosOverwatch>()) };
        let runtime = unsafe { Box::from_raw(self.runtime.cast::<Runtime>()) };
        (overwatch, runtime)
    }

    fn stop(self) -> NomosNodeErrorCode {
        let runtime_handle = self.get_runtime_handle();
        let overwatch_handle = self.get_overwatch_handle();
        if let Err(e) = runtime_handle.block_on(overwatch_handle.stop_all_services()) {
            eprintln!("Could not stop services: {e}");
            return NomosNodeErrorCode::StopError;
        }
        NomosNodeErrorCode::None
    }
}

// Implement Drop to prevent memory leaks
impl Drop for NomosNode {
    fn drop(&mut self) {
        if self.overwatch.is_null() {
            eprintln!("Attempted to drop a null overwatch pointer. This is a bug");
        }
        if self.runtime.is_null() {
            eprintln!("Attempted to drop a null tokio runtime pointer. This is a bug");
        }
        let _ = unsafe { Box::from_raw(self.overwatch.cast::<NomosOverwatch>()) };
        let _ = unsafe { Box::from_raw(self.runtime.cast::<Runtime>()) };
    }
}

#[unsafe(no_mangle)]
/// # Safety
///
/// The caller must ensure that:
/// - `node` is a valid pointer to a `NomosNode` instance
/// - The `NomosNode` instance was created by this library
/// - The pointer will not be used after this function returns
pub unsafe extern "C" fn stop_node(node: *mut NomosNode) -> NomosNodeErrorCode {
    if node.is_null() {
        eprintln!("Attempted to stop a null node pointer. This is a bug. Aborting.");
        return NomosNodeErrorCode::NullPtr;
    }

    let node = unsafe { Box::from_raw(node) };
    node.stop()
}
