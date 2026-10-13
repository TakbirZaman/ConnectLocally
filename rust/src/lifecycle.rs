//! Host window-close cleanup before the Flutter engine is destroyed.
//! The export attribute fixes the ABI name; there is no pointer dereference.
#[allow(unsafe_code)]
#[unsafe(no_mangle)]
pub extern "C" fn localconnect_shutdown() {
    let _ = std::panic::catch_unwind(|| {
        if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            runtime.block_on(async {
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(5), crate::api::shutdown())
                        .await;
            });
        }
    });
}
