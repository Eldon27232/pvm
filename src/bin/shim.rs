//! pvm-shim —— 控制台子系统转发器。复制到 shims\python.exe / pip.exe 等，
//! 按 .python-version 解析并转发到真实解释器（SPEC §7.1）。

fn main() {
    pvm::shim::run_shim();
}
