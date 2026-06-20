// pvm GUI 前端逻辑。通过 window.__TAURI__ 调用 Rust 命令并监听安装进度事件。
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const REPO_URL = "https://github.com/Eldon27232/pvm";
function lsGet(key) { try { return JSON.parse(localStorage.getItem(key)); } catch (_) { return null; } }
function lsSet(key, val) { try { localStorage.setItem(key, JSON.stringify(val)); } catch (_) {} }

const state = {
  lang: localStorage.getItem("pvm_lang") || "zh",
  theme: localStorage.getItem("pvm_theme") || "dark",
  nav: "installed",
  remoteSource: "standalone",
  remoteCache: { standalone: null, org: null },
  installed: [],
  threads: 8,
  setGlobal: true,
  pkgPy: null,
  pkgOutdated: {},
  pkgCacheByPy: {},
  interpCache: null,
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
    packages: renderPackages,
    install: renderInstall,
    venv: renderVenv,
    mirror: renderMirror,
    settings: renderSettings,
  }[state.nav])(c);
}

// ---------- 面板：已安装 ----------
async function renderInstalled(c) {
  const [list, sys] = await Promise.all([call("list_installed"), call("list_system_pythons")]);
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
  const sysRows = sys.length
    ? sys
        .map(
          (s) => `
      <div class="row">
        <div class="grow">
          <div class="vname">Python ${esc(s.version)} <span class="badge">${t("sys_" + s.origin)}</span></div>
          <div class="vmeta">${esc(s.path)}</div>
        </div>
      </div>`
        )
        .join("")
    : `<div class="empty">${t("sys_empty")}</div>`;
  c.innerHTML = `
    <div class="panel-head"><div class="panel-title">${t("nav_installed")}</div></div>
    <div class="list-scroll">${rows}</div>
    <div class="panel-head" style="margin-top:20px"><div class="panel-title" style="font-size:16px">${t("sys_title")}</div></div>
    <div class="panel-hint">${t("sys_hint")}</div>
    <div class="list-scroll">${sysRows}</div>`;

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

// ---------- 面板：包管理 ----------
let pkgCache = [];

async function renderPackages(c) {
  const mirrors = await call("mirror_list");
  let interps = state.interpCache || lsGet("interps");
  if (!interps) {
    interps = await call("list_interpreters");
    lsSet("interps", interps);
  } else {
    // 有缓存先用，后台刷新
    invoke("list_interpreters").then((fresh) => { lsSet("interps", fresh); state.interpCache = fresh; }).catch(() => {});
  }
  state.interpCache = interps;
  if (!interps.length) {
    c.innerHTML = `<div class="panel-head"><div class="panel-title">${t("nav_packages")}</div></div><div class="empty">${t("pkg_no_interp")}</div>`;
    return;
  }
  if (!state.pkgPy || !interps.some((i) => i.py_exe === state.pkgPy)) {
    const last = lsGet("lastPy");
    state.pkgPy = last && interps.some((i) => i.py_exe === last) ? last : interps[0].py_exe;
  }
  const iopts = interps
    .map((i) => `<option value="${esc(i.py_exe)}" ${i.py_exe === state.pkgPy ? "selected" : ""}>${esc(i.label)}</option>`)
    .join("");
  const mopts = `<option value="">${t("none")}</option>` + mirrors.map((m) => `<option value="${esc(m.alias)}">${esc(m.display)}</option>`).join("");

  c.innerHTML = `
    <div class="panel-head"><div class="panel-title">${t("nav_packages")}</div></div>
    <div class="card">
      <div class="inline">
        <div class="field" style="flex:1"><label>${t("pkg_interpreter")}</label><select id="pkgpy">${iopts}</select></div>
        <button class="btn" id="pkgterm" style="align-self:flex-end;margin-bottom:12px">${t("pkg_open_terminal")}</button>
      </div>
      <div class="inline">
        <div class="field" style="flex:1;min-width:160px"><label>${t("pkg_install_spec")}</label><input type="text" id="pkgspec" placeholder="requests / numpy==1.26.0"/></div>
        <div class="field" style="min-width:120px"><label>${t("venv_mirror")}</label><select id="pkgmirror">${mopts}</select></div>
        <button class="btn" id="pkgsearch2" style="align-self:flex-end;margin-bottom:12px">${t("pkg_search_pypi")}</button>
        <button class="btn" id="pkgpreview" style="align-self:flex-end;margin-bottom:12px">${t("pkg_preview")}</button>
        <button class="btn btn-primary" id="pkginstall" style="align-self:flex-end;margin-bottom:12px">${t("btn_install")}</button>
      </div>
      <div class="actions">
        <button class="btn btn-sm" id="pkgoutdated">${t("pkg_check_outdated")}</button>
        <button class="btn btn-sm" id="pkgexport">${t("pkg_export")}</button>
        <button class="btn btn-sm" id="pkgimport">${t("pkg_import")}</button>
        <button class="btn btn-sm" id="pkgrefresh">${t("btn_refresh")}</button>
      </div>
    </div>
    <div class="searchbar"><input type="text" id="pkgsearch" placeholder="${t("pkg_search")}"/></div>
    <div class="list-scroll" id="pkglist"><div class="empty"><span class="spin"></span> ${t("loading")}</div></div>`;

  const py = () => state.pkgPy;
  const mirror = () => { const el = document.getElementById("pkgmirror"); return el ? el.value || null : null; };

  document.getElementById("pkgpy").addEventListener("change", (e) => { state.pkgPy = e.target.value; lsSet("lastPy", state.pkgPy); state.pkgOutdated = {}; loadPackages(); });
  document.getElementById("pkgrefresh").addEventListener("click", () => { lsSet("pkgs:" + state.pkgPy, null); state.pkgOutdated = {}; loadPackages(); });
  document.getElementById("pkgterm").addEventListener("click", async () => { try { await invoke("open_terminal", { pyExe: state.pkgPy }); } catch (e) { toast(t("err_prefix") + e, "err"); } });
  document.getElementById("pkgsearch").addEventListener("input", (e) => paintPackages(e.target.value.trim()));
  document.getElementById("pkginstall").addEventListener("click", async () => {
    const spec = document.getElementById("pkgspec").value.trim();
    if (!spec) return;
    await pkgOp(() => invoke("pkg_install", { pyExe: py(), spec, mirror: mirror(), upgrade: false }), t("pkg_installing", { spec }));
    document.getElementById("pkgspec").value = "";
    loadPackages();
  });
  document.getElementById("pkgsearch2").addEventListener("click", showSearch);
  document.getElementById("pkgpreview").addEventListener("click", () => {
    const spec = document.getElementById("pkgspec").value.trim();
    if (spec) showDryRun(spec);
  });
  document.getElementById("pkgoutdated").addEventListener("click", async () => {
    toast(t("pkg_checking"), "");
    try {
      const od = await invoke("pkg_outdated", { pyExe: py() });
      state.pkgOutdated = {};
      od.forEach((o) => (state.pkgOutdated[o.name.toLowerCase()] = o.latest_version));
      toast(t("pkg_outdated_n", { n: od.length }), "ok");
      paintPackages(document.getElementById("pkgsearch").value.trim());
    } catch (e) { toast(t("err_prefix") + e, "err"); }
  });
  document.getElementById("pkgexport").addEventListener("click", async () => {
    try {
      const txt = await invoke("pkg_freeze", { pyExe: py() });
      await navigator.clipboard.writeText(txt);
      toast(t("pkg_exported"), "ok");
    } catch (e) { toast(t("err_prefix") + e, "err"); }
  });
  document.getElementById("pkgimport").addEventListener("click", async () => {
    const f = prompt(t("pkg_import_prompt"));
    if (!f) return;
    await pkgOp(() => invoke("pkg_install_requirements", { pyExe: py(), reqFile: f, mirror: mirror() }), t("pkg_importing"));
    loadPackages();
  });

  loadPackages();
}

async function loadPackages() {
  const box = document.getElementById("pkglist");
  const key = "pkgs:" + state.pkgPy;
  const cached = lsGet(key);
  if (cached) { pkgCache = cached; paintPackages(currentFilter()); }
  else if (box) box.innerHTML = skeletonRows();
  try {
    const fresh = await invoke("pkg_list", { pyExe: state.pkgPy });
    lsSet(key, fresh);
    pkgCache = fresh;
    paintPackages(currentFilter());
  } catch (e) {
    if (!cached && box) box.innerHTML = `<div class="empty">${t("err_prefix")}${esc(e)}</div>`;
  }
}

function paintPackages(filter) {
  const box = document.getElementById("pkglist");
  if (!box) return;
  let list = pkgCache;
  if (filter) list = list.filter((p) => p.name.toLowerCase().includes(filter.toLowerCase()));
  if (!list.length) { box.innerHTML = `<div class="empty">${t("pkg_empty")}</div>`; return; }
  box.innerHTML = list
    .map((p) => {
      const latest = state.pkgOutdated[p.name.toLowerCase()];
      return `
      <div class="row">
        <div class="grow">
          <div class="vname">${esc(p.name)} <span class="muted">${esc(p.version)}</span>
            ${latest ? `<span class="badge badge-ft">↑ ${esc(latest)}</span>` : ""}
          </div>
        </div>
        <div class="actions">
          ${latest ? `<button class="btn btn-sm btn-primary" data-pkgup="${esc(p.name)}">${t("pkg_upgrade")}</button>` : ""}
          <button class="btn btn-sm" data-pkgshow="${esc(p.name)}">${t("pkg_detail")}</button>
          <button class="btn btn-sm btn-danger" data-pkgrm="${esc(p.name)}">${t("btn_uninstall")}</button>
        </div>
      </div>`;
    })
    .join("");
  const py = () => state.pkgPy;
  const mirror = () => { const el = document.getElementById("pkgmirror"); return el ? el.value || null : null; };
  box.querySelectorAll("[data-pkgup]").forEach((b) =>
    b.addEventListener("click", async () => {
      await pkgOp(() => invoke("pkg_install", { pyExe: py(), spec: b.dataset.pkgup, mirror: mirror(), upgrade: true }), t("pkg_upgrading", { name: b.dataset.pkgup }));
      delete state.pkgOutdated[b.dataset.pkgup.toLowerCase()];
      loadPackages();
    })
  );
  box.querySelectorAll("[data-pkgrm]").forEach((b) =>
    b.addEventListener("click", async () => {
      if (!confirm(t("pkg_confirm_rm", { name: b.dataset.pkgrm }))) return;
      await pkgOp(() => invoke("pkg_uninstall", { pyExe: py(), name: b.dataset.pkgrm }), t("pkg_removing", { name: b.dataset.pkgrm }));
      loadPackages();
    })
  );
  box.querySelectorAll("[data-pkgshow]").forEach((b) =>
    b.addEventListener("click", () => showDetail(b.dataset.pkgshow))
  );
}

async function pkgOp(fn, runningMsg) {
  toast(runningMsg, "");
  try { await fn(); toast(t("done"), "ok"); lsSet("pkgs:" + state.pkgPy, null); } catch (e) { toast(t("err_prefix") + e, "err"); }
}

function currentFilter() { const el = document.getElementById("pkgsearch"); return el ? el.value.trim() : ""; }
function curMirror() { const el = document.getElementById("pkgmirror"); return el ? el.value || null : null; }
function skeletonRows() {
  return Array.from({ length: 9 }).map(() => `<div class="row skeleton"><div class="grow"><div class="sk-line"></div></div></div>`).join("");
}

// ---------- 富详情模态（本地秒出 + PyPI 异步补充）----------
async function showDetail(name) {
  openModal(`<div class="empty"><span class="spin"></span> ${t("loading")}</div>`);
  let local;
  try {
    local = await invoke("pkg_detail", { pyExe: state.pkgPy, name });
  } catch (e) {
    openModal(`<div class="detail-head"><div class="detail-title">${esc(name)}</div><button class="icon-btn" id="modal-close">✕</button></div><div class="empty">${t("err_prefix")}${esc(e)}</div>`);
    const cb = document.getElementById("modal-close"); if (cb) cb.addEventListener("click", closeModal);
    return;
  }
  renderDetail(local, null); // 本地秒出
  invoke("pkg_pypi", { name }).then((pypi) => {
    const m = document.getElementById("modal");
    if (m && m.style.display !== "none") renderDetail(local, pypi); // 补 PyPI（可用版本/README/链接）
  }).catch(() => {});
}

function renderDetail(d, pypi) {
  const summary = d.summary || (pypi && pypi.summary) || "";
  const author = d.author || (pypi && pypi.author) || "";
  const license = d.license || (pypi && pypi.license) || "";
  const links = [];
  if (d.home_page) links.push([t("dt_home"), d.home_page]);
  else if (pypi && pypi.home_page) links.push([t("dt_home"), pypi.home_page]);
  if (pypi) (pypi.project_urls || []).forEach(([k, u]) => links.push([k, u]));
  const linkHtml = links.map(([k, u]) => `<a class="lnk" data-url="${esc(u)}">${esc(k)}</a>`).join(" · ");
  const chips = (arr) => (arr && arr.length) ? arr.map((x) => `<span class="badge dep" data-dep="${esc(x)}">${esc(x)}</span>`).join(" ") : `<span class="muted">${t("none")}</span>`;
  let verBlock;
  if (pypi) {
    const verOpts = (pypi.available_versions || []).slice(0, 400).map((v) => `<option value="${esc(v)}">${esc(v)}</option>`).join("");
    verBlock = `<div class="inline" style="margin:12px 0">
        <div class="field" style="min-width:160px"><label>${t("dt_versions")}</label><select id="dt-ver">${verOpts || `<option>—</option>`}</select></div>
        <button class="btn btn-primary" id="dt-install" style="align-self:flex-end;margin-bottom:12px">${t("dt_install_ver")}</button>
        <button class="btn" id="dt-copy" style="align-self:flex-end;margin-bottom:12px">${t("dt_copy_install")}</button>
      </div>`;
  } else {
    verBlock = `<div class="muted" style="margin:12px 0"><span class="spin"></span> ${t("dt_loading_pypi")}</div>`;
  }
  const readme = pypi ? pypi.readme : "";
  openModal(`
    <div class="detail-head">
      <div>
        <div class="detail-title">${esc(d.name)} <span class="muted">${esc(d.version)}</span></div>
        <div class="muted">${esc(summary)}</div>
      </div>
      <button class="icon-btn" id="modal-close">✕</button>
    </div>
    <div class="detail-body">
      <div class="kv"><span class="k">${t("dt_author")}</span><span>${esc(author || t("none"))}</span></div>
      <div class="kv"><span class="k">${t("dt_license")}</span><span>${esc(license || t("none"))}</span></div>
      <div class="kv"><span class="k">${t("dt_location")}</span><span>${esc(d.location || t("none"))}</span></div>
      ${linkHtml ? `<div class="kv"><span class="k">${t("dt_links")}</span><span>${linkHtml}</span></div>` : ""}
      <div class="kv"><span class="k">Requires</span><span>${chips(d.requires)}</span></div>
      <div class="kv"><span class="k">Required-by</span><span>${chips(d.required_by)}</span></div>
      ${verBlock}
      ${readme ? `<div class="detail-readme"><div class="k">${t("dt_readme")}</div><pre>${esc(readme.slice(0, 8000))}</pre></div>` : ""}
    </div>`);
  document.getElementById("modal-close").addEventListener("click", closeModal);
  const di = document.getElementById("dt-install");
  if (di) di.addEventListener("click", async () => {
    const v = document.getElementById("dt-ver").value;
    if (!v) return;
    closeModal();
    await pkgOp(() => invoke("pkg_install", { pyExe: state.pkgPy, spec: `${d.name}==${v}`, mirror: curMirror(), upgrade: true }), t("pkg_installing", { spec: `${d.name}==${v}` }));
    loadPackages();
  });
  const dc = document.getElementById("dt-copy");
  if (dc) dc.addEventListener("click", () => { navigator.clipboard.writeText(`pip install ${d.name}`); toast(t("copied"), "ok"); });
  document.querySelectorAll("#modal .lnk").forEach((a) => a.addEventListener("click", () => openExt(a.dataset.url)));
  document.querySelectorAll("#modal .dep").forEach((s) => s.addEventListener("click", () => showDetail(s.dataset.dep)));
}

function openModal(html) {
  let m = document.getElementById("modal");
  if (!m) {
    m = document.createElement("div");
    m.id = "modal";
    m.className = "modal";
    document.body.appendChild(m);
    m.addEventListener("click", (e) => { if (e.target === m) closeModal(); });
  }
  m.innerHTML = `<div class="modal-card">${html}</div>`;
  m.style.display = "flex";
}
function closeModal() { const m = document.getElementById("modal"); if (m) m.style.display = "none"; }
async function openExt(url) { try { await invoke("open_url", { url }); } catch (e) { try { await navigator.clipboard.writeText(url); toast(t("copied"), "ok"); } catch (_) {} } }

// ---------- PyPI 在线搜索 ----------
async function showSearch() {
  openModal(`
    <div class="detail-head"><div class="detail-title">${t("pkg_search_pypi")}</div><button class="icon-btn" id="modal-close">✕</button></div>
    <div class="searchbar"><input type="text" id="srch-q" placeholder="${t("pkg_search_ph")}"/><button class="btn btn-primary" id="srch-go">${t("pkg_search_pypi")}</button></div>
    <div class="list-scroll" id="srch-list" style="max-height:50vh"><div class="muted">${t("pkg_search_hint")}</div></div>`);
  document.getElementById("modal-close").addEventListener("click", closeModal);
  const run = async () => {
    const q = document.getElementById("srch-q").value.trim();
    if (!q) return;
    const box = document.getElementById("srch-list");
    box.innerHTML = `<div class="empty"><span class="spin"></span> ${t("loading")}</div>`;
    try {
      const hits = await invoke("pkg_search", { query: q });
      if (!hits.length) { box.innerHTML = `<div class="empty">${t("pkg_search_none")}</div>`; return; }
      box.innerHTML = hits.map((h) => `
        <div class="row"><div class="grow"><div class="vname">${esc(h.name)}</div></div>
        <div class="actions">
          <button class="btn btn-sm" data-srch-detail="${esc(h.name)}">${t("pkg_detail")}</button>
          <button class="btn btn-sm btn-primary" data-srch-install="${esc(h.name)}">${t("btn_install")}</button>
        </div></div>`).join("");
      box.querySelectorAll("[data-srch-detail]").forEach((b) => b.addEventListener("click", () => showDetail(b.dataset.srchDetail)));
      box.querySelectorAll("[data-srch-install]").forEach((b) => b.addEventListener("click", async () => {
        const name = b.dataset.srchInstall;
        closeModal();
        await pkgOp(() => invoke("pkg_install", { pyExe: state.pkgPy, spec: name, mirror: curMirror(), upgrade: false }), t("pkg_installing", { spec: name }));
        loadPackages();
      }));
    } catch (e) { box.innerHTML = `<div class="empty">${t("err_prefix")}${esc(e)}</div>`; }
  };
  document.getElementById("srch-go").addEventListener("click", run);
  document.getElementById("srch-q").addEventListener("keydown", (e) => { if (e.key === "Enter") run(); });
  document.getElementById("srch-q").focus();
}

// ---------- 安装前 dry-run 预览 ----------
async function showDryRun(spec) {
  openModal(`
    <div class="detail-head"><div class="detail-title">${t("pkg_preview")} · ${esc(spec)}</div><button class="icon-btn" id="modal-close">✕</button></div>
    <div id="dry-body"><div class="empty"><span class="spin"></span> ${t("loading")}</div></div>`);
  document.getElementById("modal-close").addEventListener("click", closeModal);
  try {
    const changes = await invoke("pkg_dry_run", { pyExe: state.pkgPy, spec, mirror: curMirror() });
    const body = document.getElementById("dry-body");
    if (!changes.length) { body.innerHTML = `<div class="muted">${t("pkg_dry_none")}</div>`; return; }
    body.innerHTML = `<div class="panel-hint">${t("pkg_dry_will")}（${changes.length}）</div>` +
      changes.map((c) => `<div class="kv"><span>${esc(c)}</span></div>`).join("") +
      `<div class="actions" style="margin-top:12px"><button class="btn btn-primary" id="dry-install">${t("btn_install")}</button></div>`;
    document.getElementById("dry-install").addEventListener("click", async () => {
      closeModal();
      await pkgOp(() => invoke("pkg_install", { pyExe: state.pkgPy, spec, mirror: curMirror(), upgrade: false }), t("pkg_installing", { spec }));
      loadPackages();
    });
  } catch (e) {
    const body = document.getElementById("dry-body");
    if (body) body.innerHTML = `<div class="empty">${t("err_prefix")}${esc(e)}</div>`;
  }
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
      <div class="kv"><span class="k">${t("diag_proxy")}</span><span>${esc(doc.proxy)}</span></div>
      <div class="actions" style="margin-top:12px"><button class="btn btn-primary" id="initbtn">${t("btn_init")}</button></div>
    </div>
    <div class="card">
      <div class="panel-hint">${t("settings_about")}</div>
      <div class="muted">${t("about_text")}</div>
      <div class="actions" style="margin-top:10px"><button class="btn" id="gh-btn">${t("github_repo")}</button></div>
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
  document.getElementById("gh-btn").addEventListener("click", () => openExt(REPO_URL));
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
  const ghTop = document.getElementById("gh-top");
  if (ghTop) ghTop.addEventListener("click", () => openExt(REPO_URL));
  prewarm();
}

// 启动预热：后台预跑解释器列表与默认解释器包列表，填 localStorage 缓存，进面板即秒开。
function prewarm() {
  invoke("list_interpreters").then((interps) => {
    lsSet("interps", interps);
    state.interpCache = interps;
    if (interps.length) {
      const last = lsGet("lastPy");
      const py = last && interps.some((i) => i.py_exe === last) ? last : interps[0].py_exe;
      invoke("pkg_list", { pyExe: py }).then((pkgs) => lsSet("pkgs:" + py, pkgs)).catch(() => {});
    }
  }).catch(() => {});
}

init();
