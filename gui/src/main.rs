//! pvm GUI（Tauri）后端入口。命令实现见 commands 模块，全部复用 pvm core。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

/// 启动时若 config 指定了代理模式且进程未设 PVM_PROXY，则注入，供 net::agent 使用。
fn set_proxy_from_config() {
    if std::env::var("PVM_PROXY").is_ok() {
        return;
    }
    if let Ok(p) = pvm::paths::Paths::discover(None) {
        if let Ok(cfg) = pvm::config::Config::load(&p) {
            if let Some(proxy) = cfg.proxy {
                if !proxy.trim().is_empty() {
                    std::env::set_var("PVM_PROXY", proxy);
                }
            }
        }
    }
}

fn main() {
    set_proxy_from_config();
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
            commands::pkg_freeze,
            commands::pkg_install_requirements,
            commands::pkg_detail,
            commands::pkg_pypi,
            commands::open_terminal,
            commands::open_url,
            commands::pkg_search,
            commands::pkg_dry_run,
            commands::pkg_batch,
            commands::snapshot_save,
            commands::snapshot_list,
            commands::snapshot_delete,
            commands::snapshot_apply,
            commands::scaffold,
            commands::set_proxy,
            commands::get_ai_config,
            commands::set_ai_config,
            commands::ai_diagnose,
            commands::ai_find_packages,
            commands::ai_chat,
            commands::app_version,
            commands::check_update,
            commands::download_and_run_update,
            commands::osv_scan,
            commands::path_diag,
            commands::pkg_health,
            commands::pkg_dep_graph,
            commands::uv_status,
            commands::set_use_uv,
        ])
        .run(tauri::generate_context!())
        .expect("运行 pvm GUI 失败");
}
