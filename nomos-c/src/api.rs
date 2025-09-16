use std::ffi::c_void;

use nomos_node::RuntimeServiceId;
use overwatch::overwatch::Overwatch;
use tokio::runtime::Runtime;

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
            overwatch: Box::into_raw(Box::new(overwatch)) as *mut c_void,
            runtime: Box::into_raw(Box::new(runtime)) as *mut c_void,
        }
    }

    // Helper methods to safely access the inner types
    pub unsafe fn get_overwatch(&self) -> *mut NomosOverwatch {
        self.overwatch as *mut NomosOverwatch
    }

    pub unsafe fn get_runtime(&self) -> *mut Runtime {
        self.runtime as *mut Runtime
    }

    // Helper to safely take ownership back
    pub unsafe fn into_parts(self) -> (Box<NomosOverwatch>, Box<Runtime>) {
        let overwatch = unsafe { Box::from_raw(self.overwatch as *mut NomosOverwatch) };
        let runtime = unsafe { Box::from_raw(self.runtime as *mut Runtime) };
        (overwatch, runtime)
    }
}

// Implement Drop to prevent memory leaks
impl Drop for NomosNode {
    fn drop(&mut self) {
        if !self.overwatch.is_null() {
            let _ = unsafe { Box::from_raw(self.overwatch as *mut NomosOverwatch) };
        }
        if !self.runtime.is_null() {
            let _ = unsafe { Box::from_raw(self.runtime as *mut Runtime) };
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn stop_node(node: *mut NomosNode) {
    if node.is_null() {
        return;
    }

    unsafe {
        let node = Box::from_raw(node);
        // The Drop implementation will clean up the internal pointers
        drop(node);
    }
}
