//! pvm 可执行入口。

use clap::Parser;
use pvm::cli::Cli;

fn main() {
    let cli = Cli::parse();
    if let Err(e) = pvm::commands::run(cli) {
        eprintln!("错误: {e}");
        std::process::exit(e.exit_code());
    }
}
