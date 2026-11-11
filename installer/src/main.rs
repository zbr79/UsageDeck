// No console window in release builds, so a pass-through run shows nothing of its own.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod decide;
mod system;
mod ui;

use decide::Launch;

fn main() {
    let command_line = system::command_line();
    let args = decide::command_line_args(&command_line);
    let nsis_args = match decide::launch(args, system::webview2_installed()) {
        Launch::CustomUi => return ui::run(),
        Launch::NsisUi => "",
        Launch::PassThrough(args) => args,
    };
    std::process::exit(system::run_nsis(nsis_args).unwrap_or(1));
}
