// pvm GUI 前端逻辑。通过 window.__TAURI__ 调用 Rust 命令并监听安装进度事件。
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = {
  lang: localStorage.getItem("pvm_lang") || "zh",
  theme: localStorage.getItem("pvm_theme") || "dark",
  nav: "installed",
  remoteSource: "standalone",
  remoteCache: { standalone: null, org: null },
  installed: [],
  threads: 8,
  setGlobal: true,
};
// 安装进度状态：id -> {downloaded,total,stage,done,success,error}
const progress = {};

function t(key, vars) {
  let s = (window.I18N[state.lang] && window.I18N[state.lang][key]) || key;
  if (vars) for (const k in vars) s = s.replace("{" + k + "}", vars[k]);
  return s;
}
function esc(s) {
  return String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
}
function fmtSize(b) {
  if (!b) return "—";
  const u = ["B", "KB", "MB", "GB"];
  let i = 0, n = b;
  while (n >= 1024 && i < u.length - 1) { n /= 1024; i++; }
  return n.toFixed(n < 10 && i > 0 ? 1 : 0) + " " + u[i];
}
let toastTimer;
function toast(msg, kind) {
  const el = document.getElementById("toast");
  el.textContent = msg;
  el.className = "toast " + (kind || "");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.add("hidden"), 3200);
}
async function call(cmd, args) {
  try {
    return await invoke(cmd, args || {});
  } catch (e) {
    toast(t("err_prefix") + e, "err");
    throw e;
  }
}

// ---------- 主题 / 语言 ----------
function applyTheme() {
  document.documentElement.setAttribute("data-theme", state.theme);
  document.getElementById("theme-btn").textContent = state.theme === "dark" ? "◐" : "◑";
}
function applyStatic() {
  document.documentElement.setAttribute("lang", state.lang === "zh" ? "zh-CN" : "en");
  document.getElementById("app-title").textContent = t("app_title");
  document.getElementById("lang-btn").textContent = state.lang === "zh" ? "EN" : "中文";
  document.querySelectorAll(".nav-item").forEach((b) => {
    b.textContent = t("nav_" + b.dataset.nav);
    b.classList.toggle("active", b.dataset.nav === state.nav);
  });
}

// ---------- 当前生效版本 ----------
async function refreshCurrent() {
  const badge = document.getElementById("current-badge");
  try {
    const cur = await invoke("current_version");
    if (cur) {
      badge.textContent = `${t("current_label")}: ${cur.version} · ${t("from_" + cur.from)}`;
      badge.style.display = "";
    } else {
      badge.textContent = t("current_none");
      badge.style.display = "";
    }
  } catch {
    badge.style.display = "none";
  }
}

// ---------- 路由 ----------
function render() {
  applyStatic();
  const c = document.getElementById("content");
  c.innerHTML = `<div class="empty"><span class="spin"></span> ${t("loading")}</div>`;
  ({
    installed: renderInstalled,
    install: renderInstall,
    venv: renderVenv,
    mirror: renderMirror,
    settings: renderSettings,
  }[state.nav])(c);
}

// ---------- 面板：已安装 ----------
async function renderInstalled(c) {
  const list = await call("list_installed");
  state.installed = list;
  let rows = list
    .map(
      (v) => `
    <div class="row" data-id="${esc(v.id)}">
      <div class="grow">
        <div class="vname">${esc(v.version)}
          ${v.is_global ? `<span class="badge badge-global">${t("badge_global")}</span>` : ""}
          ${v.freethreaded ? `<span class="badge badge-ft">free-threaded</span>` : ""}
        </div>
        <div class="vmeta">${esc(v.source)} · ${esc(v.path)}</div>
      </div>
      <div class="actions">
        ${v.is_global ? "" : `<button class="btn btn-sm" data-act="global" data-id="${esc(v.id)}">${t("btn_set_global")}</button>`}
        <button class="btn btn-sm" data-act="local" data-id="${esc(v.id)}">${t("btn_set_local")}</button>
        <button class="btn btn-sm btn-danger" data-act="uninstall" data-id="${esc(v.id)}">${t("btn_uninstall")}</button>
      </div>
    </div>`
    )
    .join("");
  if (!list.length) rows = `<div class="empty">${t("installed_empty")}</div>`;
  c.innerHTML = `
    <div class="panel-head"><div class="panel-title">${t("nav_installed")}</div></div>
    <div class="list-scroll">${rows}</div>`;

  c.querySelectorAll("[data-act]").forEach((b) =>
    b.addEventListener("click", async () => {
      const id = b.dataset.id;
      if (b.dataset.act === "global") {
        await call("set_global", { id });
        toast(t("done"), "ok");
        render();
        refreshCurrent();
      } else if (b.dataset.act === "uninstall") {
        if (!confirm(t("confirm_uninstall", { id }))) return;
        await call("uninstall", { id });
        toast(t("done"), "ok");
        render();
        refreshCurrent();
      } else if (b.dataset.act === "local") {
        const dir = prompt(t("local_dir_prompt"));
        if (!dir) return;
        await call("set_local", { id, dir });
        toast(t("done"), "ok");
        refreshCurrent();
      }
    })
  );
}

// ---------- 面板：安装新版本 ----------
async function renderInstall(c) {
  c.innerHTML = `
    <div class="panel-head">
      <div class="panel-title">${t("nav_install")}</div>
      <div class="seg" id="src-seg">
        <button data-src="standalone" class="${state.remoteSource === "standalone" ? "active" : ""}">${t("source_standalone")}</button>
        <button data-src="org" class="${state.remoteSource === "org" ? "active" : ""}">${t("source_org")}</button>
      </div>
    </div>
    <div class="panel-hint">${t("remote_hint")}</div>
    <div class="searchbar">
      <input type="text" id="search" placeholder="${t("search_ph")}" />
      <label class="check"><span>${t("install_threads")}</span>
        <select id="threads">${[2,4,8,12,16].map((n)=>`<option value="${n}" ${n===state.threads?"selected":""}>${n}</option>`).join("")}</select>
      </label>
      <label class="check"><input type="checkbox" id="setglobal" ${state.setGlobal?"checked":""}/> ${t("install_set_global")}</label>
      <button class="btn" id="refresh">${t("btn_refresh")}</button>
    </div>
    <div class="list-scroll" id="remote-list"><div class="empty"><span class="spin"></span> ${t("loading")}</div></div>`;

  document.getElementById("threads").addEventListener("change", (e) => (state.threads = +e.target.value));
  document.getElementById("setglobal").addEventListener("change", (e) => (state.setGlobal = e.target.checked));
  c.querySelectorAll("#src-seg button").forEach((b) =>
    b.addEventListener("click", () => { state.remoteSource = b.dataset.src; render(); })
  );
  document.getElementById("refresh").addEventListener("click", () => loadRemote(true));
  document.getElementById("search").addEventListener("input", (e) => paintRemote(e.target.value.trim()));
  loadRemote(false);
}

async function loadRemote(refresh) {
  const src = state.remoteSource;
  const box = document.getElementById("remote-list");
  if (box) box.innerHTML = `<div class="empty"><span class="spin"></span> ${t("loading")}</div>`;
  try {
    const list = await invoke("list_remote", { source: src, refresh });
    state.remoteCache[src] = list;
    paintRemote("");
  } catch (e) {
    if (box) box.innerHTML = `<div class="empty">${t("err_prefix")}${esc(e)}</div>`;
  }
}

function paintRemote(filter) {
  const src = state.remoteSource;
  const box = document.getElementById("remote-list");
  if (!box) return;
  let list = state.remoteCache[src] || [];
  if (filter) list = list.filter((r) => r.version.includes(filter));
  list = list.slice(0, 120);
  if (!list.length) { box.innerHTML = `<div class="empty">—</div>`; return; }
  box.innerHTML = list
    .map((r) => {
      const id = `cpython-${r.version}-${r.source}`;
      const p = progress[id];
      return `
      <div class="row" data-id="${esc(id)}">
        <div class="grow">
          <div class="vname">${esc(r.version)}
            ${r.freethreaded ? `<span class="badge badge-ft">free-threaded</span>` : ""}
          </div>
          <div class="vmeta">${esc(r.source)}${r.date ? " · " + esc(r.date) : ""}${r.size ? " · " + fmtSize(r.size) : ""}</div>
          ${p ? progressHtml(p) : ""}
        </div>
        <div class="actions">
          ${
            r.installed
              ? `<span class="badge badge-global">${t("btn_installed")}</span>`
              : `<button class="btn btn-sm btn-primary" data-install="${esc(r.version)}" data-src="${esc(r.source)}">${t("btn_install")}</button>`
          }
        </div>
      </div>`;
    })
    .join("");

  box.querySelectorAll("[data-install]").forEach((b) =>
    b.addEventListener("click", () => startInstall(b.dataset.install, b.dataset.src, b))
  );
}

function progressHtml(p) {
  if (p.done) {
    return `<div class="prog-label">${p.success ? "✓ " + t("done") : "✕ " + t("failed") + (p.error ? ": " + esc(p.error) : "")}</div>`;
  }
  let pct = p.total ? Math.floor((p.downloaded / p.total) * 100) : 0;
  let label;
  if (p.stage === "install") label = t("state_installing");
  else if (p.total) label = `${t("state_downloading")} · ${pct}% · ${fmtSize(p.downloaded)}/${fmtSize(p.total)}`;
  else label = t("state_start");
  return `<div class="progress"><div style="width:${pct}%"></div></div><div class="prog-label">${label}</div>`;
}

async function startInstall(version, source, btn) {
  const id = `cpython-${version}-${source}`;
  progress[id] = { downloaded: 0, total: 0, stage: "start", done: false };
  if (btn) btn.disabled = true;
  updateRow(id);
  try {
    await invoke("install", {
      version,
      source: source === "org" ? "cpython" : "standalone",
      freethreaded: false,
      threads: state.threads,
      setGlobal: state.setGlobal,
    });
  } catch (e) {
    progress[id] = { done: true, success: false, error: String(e) };
    updateRow(id);
  }
}

function updateRow(id) {
  const row = document.querySelector(`#remote-list .row[data-id="${CSS.escape(id)}"]`);
  if (!row) return;
  const grow = row.querySelector(".grow");
  // 移除旧进度，重绘
  grow.querySelectorAll(".progress, .prog-label").forEach((e) => e.remove());
  const p = progress[id];
  if (p) grow.insertAdjacentHTML("beforeend", progressHtml(p));
}

// 安装事件监听（全局注册一次）
function wireInstallEvents() {
  listen("install://start", (e) => {
    const id = e.payload.id;
    progress[id] = { ...(progress[id] || {}), stage: "start", done: false };
    updateRow(id);
  });
  listen("install://progress", (e) => {
    const { id, downloaded, total } = e.payload;
    progress[id] = { ...(progress[id] || {}), downloaded, total, stage: "download", done: false };
    updateRow(id);
  });
  listen("install://stage", (e) => {
    const { id, stage } = e.payload;
    progress[id] = { ...(progress[id] || {}), stage, done: false };
    updateRow(id);
  });
  listen("install://done", (e) => {
    const { id, success, error } = e.payload;
    progress[id] = { done: true, success, error };
    updateRow(id);
    if (success) {
      toast(`${id} · ${t("done")}`, "ok");
      // 刷新已安装标记
      if (state.nav === "install") loadRemote(false);
      refreshCurrent();
    } else {
      toast(`${id} · ${t("failed")}: ${error}`, "err");
    }
  });
}

// ---------- 面板：虚拟环境 ----------
async function renderVenv(c) {
  const [venvs, installed] = await Promise.all([call("venv_list"), call("list_installed")]);
  const mirrors = await call("mirror_list");
  const opts = installed.map((v) => `<option value="${esc(v.id)}">${esc(v.version)} (${esc(v.source)})</option>`).join("");
  const mopts = `<option value="">—</option>` + mirrors.map((m) => `<option value="${esc(m.alias)}">${esc(m.display)}</option>`).join("");
  const rows = venvs.length
    ? venvs
        .map(
          (v) => `
      <div class="row">
        <div class="grow">
          <div class="vname">${esc(v.name)}</div>
          <div class="vmeta">${esc(v.python_version)} · ${esc(v.path)}</div>
        </div>
        <div class="actions">
          <button class="btn btn-sm" data-copy="${esc(v.path)}">${t("copy_activate")}</button>
          <button class="btn btn-sm btn-danger" data-rmvenv="${esc(v.name)}">${t("btn_remove")}</button>
        </div>
      </div>`
        )
        .join("")
    : `<div class="empty">${t("venv_empty")}</div>`;

  c.innerHTML = `
    <div class="panel-head"><div class="panel-title">${t("nav_venv")}</div></div>
    <div class="card">
      <div class="inline">
        <div class="field" style="flex:1;min-width:140px"><label>${t("venv_name")}</label><input type="text" id="vname" placeholder="myenv"/></div>
        <div class="field" style="flex:1;min-width:160px"><label>${t("venv_pyver")}</label><select id="vpy">${opts || `<option value="">—</option>`}</select></div>
        <div class="field" style="min-width:140px"><label>${t("venv_mirror")}</label><select id="vmirror">${mopts}</select></div>
        <button class="btn btn-primary" id="vcreate" style="align-self:flex-end;margin-bottom:12px">${t("btn_create")}</button>
      </div>
    </div>
    <div class="list-scroll">${rows}</div>`;

  document.getElementById("vcreate").addEventListener("click", async () => {
    const name = document.getElementById("vname").value.trim();
    const selector = document.getElementById("vpy").value;
    const mirror = document.getElementById("vmirror").value || null;
    if (!name || !selector) { toast(t("err_prefix") + t("venv_name") + " / " + t("venv_pyver"), "err"); return; }
    await call("venv_create", { name, selector, mirror });
    toast(t("venv_created"), "ok");
    render();
  });
  c.querySelectorAll("[data-rmvenv]").forEach((b) =>
    b.addEventListener("click", async () => {
      if (!confirm(t("confirm_remove_venv", { name: b.dataset.rmvenv }))) return;
      await call("venv_remove", { name: b.dataset.rmvenv });
      toast(t("done"), "ok");
      render();
    })
  );
  c.querySelectorAll("[data-copy]").forEach((b) =>
    b.addEventListener("click", () => {
      navigator.clipboard.writeText(`& '${b.dataset.copy}\\Scripts\\Activate.ps1'`);
      toast(t("copied"), "ok");
    })
  );
}

// ---------- 面板：pip 镜像 ----------
async function renderMirror(c) {
  const [mirrors, cur] = await Promise.all([call("mirror_list"), call("mirror_current")]);
  const curText = cur.index_url ? esc(cur.index_url) : t("mirror_none");
  const items = mirrors
    .map(
      (m) => `<label class="row" style="cursor:pointer">
        <input type="radio" name="mirror" value="${esc(m.alias)}" ${cur.index_url === m.index_url ? "checked" : ""}/>
        <div class="grow"><div class="vname">${esc(m.display)}</div><div class="vmeta">${esc(m.index_url)}</div></div>
      </label>`
    )
    .join("");
  c.innerHTML = `
    <div class="panel-head"><div class="panel-title">${t("nav_mirror")}</div></div>
    <div class="card"><div class="kv"><span class="k">${t("mirror_current")}</span><span>${curText}</span></div></div>
    <div class="panel-hint">${t("mirror_builtin")}</div>
    ${items}
    <div class="card" style="margin-top:14px">
      <div class="field"><label>${t("mirror_custom")}</label><input type="text" id="customurl" placeholder="https://..."/></div>
      <div class="actions">
        <button class="btn btn-primary" id="applymirror">${t("btn_apply")}</button>
        <button class="btn" id="resetmirror">${t("btn_reset")}</button>
      </div>
    </div>`;

  document.getElementById("applymirror").addEventListener("click", async () => {
    const custom = document.getElementById("customurl").value.trim();
    const radio = c.querySelector('input[name="mirror"]:checked');
    const val = custom || (radio && radio.value);
    if (!val) return;
    await call("mirror_set", { nameOrUrl: val });
    toast(t("mirror_set_ok"), "ok");
    render();
  });
  document.getElementById("resetmirror").addEventListener("click", async () => {
    await call("mirror_reset");
    toast(t("mirror_reset_ok"), "ok");
    render();
  });
}

// ---------- 面板：设置 ----------
async function renderSettings(c) {
  const [cfg, doc] = await Promise.all([call("get_config"), call("doctor")]);
  c.innerHTML = `
    <div class="panel-head"><div class="panel-title">${t("nav_settings")}</div></div>
    <div class="card">
      <div class="panel-hint">${t("settings_general")}</div>
      <div class="field"><label>${t("settings_default_source")}</label>
        <select id="defsrc">
          <option value="standalone" ${cfg.default_source === "standalone" ? "selected" : ""}>${t("source_standalone")}</option>
          <option value="cpython" ${cfg.default_source === "cpython" ? "selected" : ""}>${t("source_org")}</option>
        </select>
      </div>
      <div class="kv"><span class="k">${t("settings_root")}</span><span>${esc(doc.root)}</span></div>
    </div>
    <div class="card">
      <div class="panel-hint">${t("settings_diag")}</div>
      <div class="kv"><span class="k">${t("diag_shim")}</span><span>${doc.shim_ready ? t("ready") : t("missing")}</span></div>
      <div class="kv"><span class="k">${t("diag_inpath")}</span><span>${doc.shims_in_path ? t("yes") : t("no")}</span></div>
      <div class="kv"><span class="k">${t("diag_global")}</span><span>${doc.global ? esc(doc.global) : t("none")}</span></div>
      <div class="kv"><span class="k">${t("diag_count")}</span><span>${doc.installed_count}</span></div>
      <div class="actions" style="margin-top:12px"><button class="btn btn-primary" id="initbtn">${t("btn_init")}</button></div>
    </div>
    <div class="card">
      <div class="panel-hint">${t("settings_about")}</div>
      <div class="muted">${t("about_text")}</div>
    </div>`;

  document.getElementById("defsrc").addEventListener("change", async (e) => {
    await call("set_default_source", { source: e.target.value });
    toast(t("done"), "ok");
  });
  document.getElementById("initbtn").addEventListener("click", async () => {
    const msg = await call("init_pvm");
    toast(msg, "ok");
    render();
  });
}

// ---------- 启动 ----------
function init() {
  applyTheme();
  applyStatic();
  wireInstallEvents();
  refreshCurrent();
  render();

  document.getElementById("theme-btn").addEventListener("click", () => {
    state.theme = state.theme === "dark" ? "light" : "dark";
    localStorage.setItem("pvm_theme", state.theme);
    applyTheme();
  });
  document.getElementById("lang-btn").addEventListener("click", () => {
    state.lang = state.lang === "zh" ? "en" : "zh";
    localStorage.setItem("pvm_lang", state.lang);
    refreshCurrent();
    render();
  });
  document.getElementById("nav").addEventListener("click", (e) => {
    const b = e.target.closest(".nav-item");
    if (!b) return;
    state.nav = b.dataset.nav;
    render();
  });
}
init();
