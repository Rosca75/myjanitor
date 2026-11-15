//! XC-3: the panic hook writes thread, location and payload to the data folder's log.
//! One test only: the hook and its target are process-wide.

mod common;

use common::*;
use mj_core::testutil::TempDir;

#[test]
fn a_panic_on_a_worker_thread_lands_in_the_log_file() {
    let f = engine_for(TempDir::new("diag-tree").unwrap());
    let log = f.app.join("data").join(mj_core::diag::LOG_NAME);
    let t = std::thread::Builder::new()
        .name("mj-test-worker".into())
        .spawn(|| {
            let _ = std::panic::catch_unwind(|| panic!("hostile pdf object 17"));
        })
        .unwrap();
    t.join().unwrap();
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.contains("PANIC") && text.contains("mj-test-worker"), "{text}");
    assert!(text.contains("hostile pdf object 17") && text.contains("diag_log.rs"), "{text}");
    assert_eq!(mj_core::diag::log_path().as_deref(), Some(log.as_path()));
    drop(f);
}
