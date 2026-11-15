//! `myjanitor` GUI binary: hands off to the library so commands stay testable.
// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    myjanitor_lib::run()
}
