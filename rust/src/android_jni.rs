//! JNI exports for lifecycle cleanup when Flutter is gone. The unsafe export
//! attribute establishes the Java ABI name; neither raw JNI pointer is read.
//! All implementation operations use safe Rust; panics cannot unwind into Java.
fn run(action: impl std::future::Future<Output = ()>) {
    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        runtime.block_on(action);
    }
}
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_example_localconnect_RustLifecycle_shutdownNative(
    _environment: *mut std::ffi::c_void,
    _receiver: *mut std::ffi::c_void,
    generation: i64,
) {
    let _ = std::panic::catch_unwind(|| run(crate::api::native_shutdown(generation as u64)));
}
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_example_localconnect_RustLifecycle_stopSessionNative(
    _environment: *mut std::ffi::c_void,
    _receiver: *mut std::ffi::c_void,
    generation: i64,
    most: i64,
    least: i64,
) {
    let id = uuid::Uuid::from_u64_pair(most as u64, least as u64).to_string();
    let _ =
        std::panic::catch_unwind(|| run(crate::api::native_stop_session(generation as u64, id)));
}
