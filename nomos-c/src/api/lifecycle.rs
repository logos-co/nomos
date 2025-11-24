use crate::{NomosNode, errors::NomosNodeErrorCode};

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
