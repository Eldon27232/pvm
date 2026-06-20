//! pvm GUI（Tauri）后端入口。命令实现见 commands 模块，全部复用 pvm core。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::root_dir,
            commands::list_installed,
            commands::current_version,
            commands::list_remote,
            commands::install,
            commands::uninstall,
            commands::set_global,
            commands::set_local,
            commands::venv_list,
            commands::venv_create,
            commands::venv_remove,
            commands::mirror_list,
            commands::mirror_current,
            commands::mirror_set,
            commands::mirror_reset,
            commands::get_config,
            commands::set_default_source,
            commands::doctor,
            commands::init_pvm,
            commands::list_system_pythons,
            commands::list_interpreters,
            commands::pkg_list,
            commands::pkg_outdated,
            commands::pkg_install,
            commands::pkg_uninstall,
            commands::pkg_show,
            commands::pkg_freeze,
            commands::pkg_install_requirements,
            commands::pkg_detail,
            commands::open_terminal,
            commands::open_url,
        ])
        .run(tauri::generate_context!())
        .expect("运行 pvm GUI 失败");
}
