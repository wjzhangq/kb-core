use std::sync::Arc;
use anyhow::Result;

/// Build a Tokio runtime limited to `max_threads` blocking threads.
pub fn build_tokio_runtime(max_threads: usize) -> Result<tokio::runtime::Runtime> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(max_threads.max(1))
        .max_blocking_threads(max_threads.max(1))
        .thread_name("kb-tokio")
        .enable_all()
        .build()?;
    Ok(rt)
}

/// Build the background (low-priority) Tokio runtime.
pub fn build_background_runtime(max_threads: usize) -> Result<tokio::runtime::Runtime> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(max_threads.max(1))
        .max_blocking_threads(max_threads.max(1))
        .thread_name("kb-bg")
        .on_thread_start(|| {
            demote_current_thread();
        })
        .enable_all()
        .build()?;
    Ok(rt)
}

/// Configure the global rayon thread pool to at most `max_threads` threads.
/// Must be called before any rayon usage; subsequent calls are no-ops.
pub fn configure_rayon(max_threads: usize) {
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(max_threads.max(1))
        .thread_name(|i| format!("kb-rayon-{i}"))
        .build_global();
}

/// Lower the scheduling priority of the calling thread (best-effort, not reversible).
pub fn demote_current_thread() {
    #[cfg(target_os = "linux")]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as u32, 10);
    }

    #[cfg(target_os = "macos")]
    unsafe {
        // QOS_CLASS_UTILITY = background-friendly
        extern "C" {
            fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
        }
        const QOS_CLASS_UTILITY: u32 = 0x11;
        pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0);
    }

    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}
