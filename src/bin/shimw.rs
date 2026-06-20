//! pvm-shimw —— GUI 子系统转发器（供 pythonw.exe 用，避免闪控制台）。

#![windows_subsystem = "windows"]

fn main() {
    pvm::shim::run_shim();
}
