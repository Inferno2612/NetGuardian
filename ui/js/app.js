const { invoke } = window.__TAURI__.core;

const TRASH_SVG = '<svg viewBox="0 0 24 24"><path d="M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13M10 11v6M14 11v6"/></svg>';

function esc(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
}

function avatarFor(name) {
  let h = 0;
  for (const ch of name) h = (h * 31 + ch.charCodeAt(0)) % 360;
  const letter = esc((name.trim()[0] || "?").toUpperCase());
  return `<span class="avatar" style="background:linear-gradient(145deg,hsl(${h} 55% 48%),hsl(${(h + 40) % 360} 55% 38%))">${letter}</span>`;
}

// ---------- Вкладки ----------
document.querySelectorAll(".tab-btn").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".tab-btn").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".panel").forEach((p) => p.classList.remove("active"));
    btn.classList.add("active");
    document.getElementById(`panel-${btn.dataset.tab}`).classList.add("active");
    // canvas внутри скрытой вкладки имеет нулевой размер — пересчитываем при показе
    if (btn.dataset.tab === "dashboard") chart._resize();
  });
});

function toast(msg) {
  const el = document.getElementById("toast");
  el.textContent = msg;
  el.classList.add("show");
  clearTimeout(toast._t);
  toast._t = setTimeout(() => el.classList.remove("show"), 3500);
}

// ---------- VPN-статус ----------
const $ = (id) => document.getElementById(id);

async function refreshVpnStatus() {
  let status;
  try {
    status = await invoke("get_vpn_status");
  } catch (e) {
    console.error(e);
    return;
  }
  const adapter = status.matched_adapter ? status.matched_adapter.name : "";
  let mode = "unknown";
  if (status.state === "Connected") mode = "on";
  else if (status.state === "Disconnected") mode = "off";
  document.body.dataset.vpn = mode;

  const texts = {
    on: {
      title: "VPN подключён",
      sub: "Трафик идёт через VPN, приложения из списка блокировки работают.",
      line: "Трафик защищён — <b>VPN подключён</b>",
    },
    off: {
      title: "VPN отключён",
      sub: "Приложения из списка блокировки закрываются, пока VPN выключен.",
      line: "Вы без защиты — <b>VPN отключён</b>",
    },
    unknown: {
      title: "Статус неизвестен",
      sub: "VPN-адаптер не найден. Проверьте вкладку «Настройки → Сетевые адаптеры».",
      line: "<b>Не удалось определить VPN</b>",
    },
  }[mode];

  $("heroTitle").textContent = texts.title;
  $("heroSub").textContent = texts.sub;
  $("heroLine").innerHTML = texts.line;
  $("chipAdapter").textContent = `Адаптер: ${adapter || "—"}`;
  $("dockTitle").textContent = `VPN: ${mode === "on" ? "подключён" : mode === "off" ? "отключён" : "неизвестно"}`;
  $("dockSub").textContent = adapter ? `${adapter} · метод: ${status.method}` : `метод: ${status.method}`;
}

// ---------- Трафик ----------
const ifaceSelect = $("ifaceSelect");
const chart = new SpeedChart($("trafficChart"));
let currentIface = null;

function fmtBps(bitsPerSec) {
  let v = bitsPerSec;
  const units = ["бит/с", "Кбит/с", "Мбит/с", "Гбит/с"];
  let i = 0;
  while (Math.abs(v) >= 1000 && i < units.length - 1) { v /= 1000; i++; }
  return `${v.toFixed(1)} ${units[i]}`;
}

function fmtBytes(n) {
  let v = n;
  const units = ["Б", "КБ", "МБ", "ГБ", "ТБ"];
  let i = 0;
  while (Math.abs(v) >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(1)} ${units[i]}`;
}

async function loadInterfaces() {
  const ifaces = await invoke("traffic_interfaces");
  const prev = ifaceSelect.value;
  ifaceSelect.innerHTML = "";
  ifaces.forEach((name) => {
    const opt = document.createElement("option");
    opt.value = name;
    opt.textContent = name;
    ifaceSelect.appendChild(opt);
  });
  if (ifaces.includes(prev)) ifaceSelect.value = prev;
  currentIface = ifaceSelect.value || null;
}

ifaceSelect.addEventListener("change", () => { currentIface = ifaceSelect.value; });

async function trafficTick() {
  try {
    await invoke("traffic_sample");
    if (!currentIface) {
      await loadInterfaces();
      if (!currentIface) return;
    }
    const snap = await invoke("traffic_snapshot", { interface: currentIface });
    const down = snap.series.map((s) => s.down_bps * 8);
    const up = snap.series.map((s) => s.up_bps * 8);
    chart.setData(down, up);

    const last = snap.series[snap.series.length - 1];
    const d = fmtBps(last ? last.down_bps * 8 : 0);
    const u = fmtBps(last ? last.up_bps * 8 : 0);
    $("curDown").textContent = d;
    $("curUp").textContent = u;
    $("dockDown").textContent = d;
    $("dockUp").textContent = u;
    $("hourRecv").textContent = fmtBytes(snap.last_hour_recv);
    $("hourSent").textContent = fmtBytes(snap.last_hour_sent);
    $("bootRecv").textContent = fmtBytes(snap.since_boot_recv);
    $("bootSent").textContent = fmtBytes(snap.since_boot_sent);
  } catch (e) {
    console.error(e);
  }
}

// ---------- "Как меня видят" ----------
$("refreshIp").addEventListener("click", async () => {
  const body = $("ipBody");
  body.innerHTML = `<p class="muted">Загрузка…</p>`;
  try {
    const info = await invoke("get_public_ip");
    const rows = [
      ["Страна", info.country],
      ["Регион", info.region],
      ["Город", info.city],
      ["Провайдер", info.isp],
      ["Организация", info.org],
      ["AS", info.asn],
      ["Часовой пояс", info.timezone],
    ];
    body.innerHTML =
      `<div class="ip-hero">${esc(info.ip)}</div>` +
      `<div class="kv-grid">${rows.map(([k, v]) => `<div class="kv"><span>${k}</span><b>${esc(v || "—")}</b></div>`).join("")}</div>`;
  } catch (e) {
    body.innerHTML = `<p class="muted">Ошибка: ${esc(e)}</p>`;
  }
});

// ---------- Сайты ----------
const sitesList = $("sitesList");

function statusBadge(isUp) {
  if (isUp === null || isUp === undefined) return `<span class="badge unknown">ожидание</span>`;
  return isUp ? `<span class="badge up">доступен</span>` : `<span class="badge down">недоступен</span>`;
}

async function refreshSites() {
  const sites = await invoke("sites_list");
  if (sites.length === 0) {
    sitesList.innerHTML = `<div class="empty">Сайтов пока нет — добавьте первый выше.</div>`;
    return;
  }
  sitesList.innerHTML = "";
  sites.forEach((s) => {
    const row = document.createElement("div");
    row.className = "row row-sites";
    const checked = s.last_checked ? new Date(s.last_checked * 1000).toLocaleTimeString("ru-RU") : "—";
    row.innerHTML = `
      <span class="cell-main">${avatarFor(s.url.replace(/^https?:\/\//, ""))}<span class="t" title="${esc(s.url)}">${esc(s.url.replace(/^https?:\/\//, ""))}</span></span>
      <span>${statusBadge(s.is_up)}</span>
      <span class="cell-dim">${s.status_code ?? "—"}</span>
      <span class="cell-dim">${s.response_ms ? s.response_ms.toFixed(0) + " мс" : "—"}</span>
      <span class="cell-dim">${checked}</span>
      <button class="btn icon danger-hover" title="Удалить" data-remove="${esc(s.url)}">${TRASH_SVG}</button>`;
    sitesList.appendChild(row);
  });
  sitesList.querySelectorAll("[data-remove]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      await invoke("sites_remove", { url: btn.dataset.remove });
      refreshSites();
    });
  });
}

async function addSite() {
  const input = $("siteUrlInput");
  const url = input.value.trim();
  if (!url) return;
  const ok = await invoke("sites_add", { url });
  if (!ok) {
    toast("Такой адрес уже в списке или он указан неверно");
    return;
  }
  input.value = "";
  await refreshSites();
  // сразу проверяем новый сайт, не дожидаясь планового таймера
  invoke("sites_tick").then(refreshSites).catch(console.error);
}
$("addSiteBtn").addEventListener("click", addSite);
$("siteUrlInput").addEventListener("keydown", (e) => { if (e.key === "Enter") addSite(); });

// ---------- Настройки ----------
const chkAutostart = $("chkAutostart");

async function loadSettings() {
  chkAutostart.checked = await invoke("is_autostart_set");
}

chkAutostart.addEventListener("change", async () => {
  await invoke("set_autostart", { enable: chkAutostart.checked });
  toast(chkAutostart.checked ? "Автозапуск включён" : "Автозапуск выключен");
});

$("showAdapters").addEventListener("click", async () => {
  const list = $("adaptersList");
  list.innerHTML = `<div class="empty">Загрузка…</div>`;
  try {
    const adapters = await invoke("list_adapters");
    list.innerHTML = `<div class="row row-adapters head"><span>Имя</span><span>Описание</span><span>Статус</span></div>`;
    adapters.forEach((a) => {
      const row = document.createElement("div");
      row.className = "row row-adapters";
      row.innerHTML = `
        <span class="cell-dim" style="color:var(--text)" title="${esc(a.name)}">${esc(a.name)}</span>
        <span class="cell-dim" title="${esc(a.description)}">${esc(a.description)}</span>
        <span>${a.is_up ? '<span class="badge up">активен</span>' : '<span class="badge down">неактивен</span>'}</span>`;
      list.appendChild(row);
    });
  } catch (e) {
    list.innerHTML = `<div class="empty">Ошибка: ${esc(e)}</div>`;
  }
});

// ---------- Блокировка приложений ----------
const appsList = $("appsList");
const adminHint = $("adminHint");

async function refreshAdminHint() {
  const isAdmin = await invoke("is_admin");
  if (isAdmin) {
    adminHint.innerHTML = `<span class="chip ok">Запущено от администратора</span> Надёжный режим доступен.`;
  } else {
    adminHint.innerHTML = `<span class="chip bad">Без прав администратора</span> Надёжный режим недоступен. <button class="btn" id="relaunchAdminBtn" style="margin-left:6px">Перезапустить от администратора</button>`;
    $("relaunchAdminBtn").addEventListener("click", async () => {
      try {
        await invoke("relaunch_as_admin");
        toast("Подтвердите запрос UAC, затем закройте это окно");
      } catch (e) {
        toast("Не удалось перезапустить: " + e);
      }
    });
  }
}

async function refreshApps() {
  const apps = await invoke("blocked_apps_list");
  const active = apps.filter((a) => a.enabled).length;
  $("chipApps").textContent = `Под блокировкой: ${active}`;
  $("appsCount").textContent = apps.length ? `Всего: ${apps.length}, активно: ${active}` : "";

  if (apps.length === 0) {
    appsList.innerHTML = `<div class="empty">Список пуст. Добавьте приложение, которое нельзя запускать без VPN.</div>`;
    return;
  }
  appsList.innerHTML = `<div class="row row-apps head"><span>Приложение</span><span>Файл</span><span>Простая</span><span>Надёжная</span><span></span></div>`;
  apps.forEach((a) => {
    const row = document.createElement("div");
    row.className = "row row-apps";
    row.innerHTML = `
      <span class="cell-main">${avatarFor(a.display_name)}<span class="t">${esc(a.display_name)}</span></span>
      <span class="cell-dim">${esc(a.exe_name)}</span>
      <label class="switch"><input type="checkbox" data-toggle="${esc(a.exe_name)}" ${a.enabled ? "checked" : ""} /><span></span></label>
      <label class="switch"><input type="checkbox" data-ifeo="${esc(a.exe_name)}" ${a.ifeo_enabled ? "checked" : ""} /><span></span></label>
      <button class="btn icon danger-hover" title="Удалить" data-remove-app="${esc(a.exe_name)}">${TRASH_SVG}</button>`;
    appsList.appendChild(row);
  });

  appsList.querySelectorAll("[data-toggle]").forEach((chk) => {
    chk.addEventListener("change", async () => {
      await invoke("blocked_apps_set_enabled", { exeName: chk.dataset.toggle, enabled: chk.checked });
      refreshApps();
    });
  });

  appsList.querySelectorAll("[data-ifeo]").forEach((chk) => {
    chk.addEventListener("change", async () => {
      try {
        if (chk.checked) {
          await invoke("blocked_apps_enable_ifeo", { exeName: chk.dataset.ifeo });
          toast("Надёжная блокировка включена");
        } else {
          await invoke("blocked_apps_disable_ifeo", { exeName: chk.dataset.ifeo });
          toast("Надёжная блокировка выключена");
        }
      } catch (e) {
        chk.checked = !chk.checked;
        toast(String(e));
      }
    });
  });

  appsList.querySelectorAll("[data-remove-app]").forEach((btn) => {
    btn.addEventListener("click", async () => {
      await invoke("blocked_apps_remove", { exeName: btn.dataset.removeApp });
      refreshApps();
    });
  });
}

async function addApp() {
  const exeInput = $("appExeInput");
  const nameInput = $("appDisplayInput");
  const exeName = exeInput.value.trim();
  const displayName = nameInput.value.trim() || exeName;
  if (!exeName) return;
  try {
    await invoke("blocked_apps_add", { exeName, displayName });
    exeInput.value = "";
    nameInput.value = "";
    refreshApps();
  } catch (e) {
    toast(String(e));
  }
}
$("addAppBtn").addEventListener("click", addApp);
$("appDisplayInput").addEventListener("keydown", (e) => { if (e.key === "Enter") addApp(); });
$("appExeInput").addEventListener("keydown", (e) => { if (e.key === "Enter") addApp(); });

// ---------- Блокировка доменов (временно отключена в разметке) ----------
// Разметка вкладки закомментирована в index.html; код оставлен, чтобы
// вернуть функцию, достаточно раскомментировать блок в HTML.
const domainsList = $("domainsList");
async function refreshDomains() {
  if (!domainsList) return;
  const domains = await invoke("blocked_domains_list");
  domainsList.innerHTML = "";
  domains.forEach((d) => {
    const row = document.createElement("div");
    row.className = "row";
    row.style.gridTemplateColumns = "1fr 60px 44px";
    row.innerHTML = `
      <span class="t">${esc(d.domain)}</span>
      <label class="switch"><input type="checkbox" data-toggle-domain="${esc(d.domain)}" ${d.enabled ? "checked" : ""} /><span></span></label>
      <button class="btn icon danger-hover" data-remove-domain="${esc(d.domain)}">${TRASH_SVG}</button>`;
    domainsList.appendChild(row);
  });
  domainsList.querySelectorAll("[data-toggle-domain]").forEach((chk) => {
    chk.addEventListener("change", () => invoke("blocked_domains_set_enabled", { domain: chk.dataset.toggleDomain, enabled: chk.checked }));
  });
  domainsList.querySelectorAll("[data-remove-domain]").forEach((btn) => {
    btn.addEventListener("click", async () => { await invoke("blocked_domains_remove", { domain: btn.dataset.removeDomain }); refreshDomains(); });
  });
}
if ($("addDomainBtn")) {
  $("addDomainBtn").addEventListener("click", async () => {
    const input = $("domainInput");
    const domain = input.value.trim();
    if (!domain) return;
    try { await invoke("blocked_domains_add", { domain }); input.value = ""; refreshDomains(); }
    catch (e) { toast(String(e)); }
  });
}

// ---------- Запуск ----------
async function init() {
  const steps = [loadInterfaces, loadSettings, refreshVpnStatus, refreshSites, trafficTick, refreshAdminHint, refreshApps, refreshDomains];
  for (const step of steps) {
    try { await step(); } catch (e) { console.error(step.name, e); }
  }
  setInterval(trafficTick, 1000);
  setInterval(refreshVpnStatus, 3000);
  setInterval(async () => {
    try { await invoke("sites_tick"); await refreshSites(); } catch (e) { console.error(e); }
  }, 20000);
}

init();
