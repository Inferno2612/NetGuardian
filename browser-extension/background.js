// Фоновый сервис-воркер расширения NetGuardian.
// Держит соединение с нативным хостом (netguardian_native_host.exe) через
// Native Messaging, периодически опрашивает статус VPN и список
// заблокированных доменов, и перехватывает переходы на них, пока VPN
// выключен.

const NATIVE_HOST_ID = "ru.netguardian.nativehost";
const POLL_INTERVAL_MS = 3000;

let port = null;
// null = статус ещё не получен от хоста (например, NetGuardian не запущен).
// В этом состоянии мы НЕ блокируем ничего — лучше пропустить переход,
// чем ошибочно заблокировать сайт, когда само приложение недоступно.
let vpnConnected = null;
let blockedDomains = [];

function connectNative() {
  try {
    port = chrome.runtime.connectNative(NATIVE_HOST_ID);
    port.onMessage.addListener(onNativeMessage);
    port.onDisconnect.addListener(() => {
      port = null;
      vpnConnected = null;
      if (chrome.runtime.lastError) {
        console.warn("NetGuardian native host:", chrome.runtime.lastError.message);
      }
    });
  } catch (e) {
    console.warn("Не удалось подключиться к нативному хосту NetGuardian:", e);
    port = null;
  }
}

function onNativeMessage(msg) {
  if (typeof msg.connected === "boolean") {
    vpnConnected = msg.connected;
  }
  if (Array.isArray(msg.domains)) {
    blockedDomains = msg.domains.map((d) => d.toLowerCase());
  }
}

function poll() {
  if (!port) connectNative();
  if (!port) return;
  try {
    port.postMessage({ cmd: "status" });
    port.postMessage({ cmd: "get_blocked_domains" });
  } catch (e) {
    port = null;
  }
}

function hostMatchesBlocked(host) {
  const h = host.toLowerCase().replace(/^www\./, "");
  return blockedDomains.find((d) => h === d || h.endsWith("." + d));
}

chrome.webNavigation.onBeforeNavigate.addListener(
  (details) => {
    if (details.frameId !== 0) return; // только переходы в главном фрейме вкладки
    if (vpnConnected !== false) return; // блокируем только при ЯВНО известном "VPN выключен"

    let host;
    try {
      host = new URL(details.url).hostname;
    } catch {
      return;
    }

    const matched = hostMatchesBlocked(host);
    if (matched) {
      chrome.tabs.update(details.tabId, {
        url: chrome.runtime.getURL("blocked.html") + "?site=" + encodeURIComponent(matched),
      });
    }
  },
  { url: [{ schemes: ["http", "https"] }] }
);

// Основной опрос — пока сервис-воркер активен (обычно всё время, пока
// открыт хотя бы один таб с активностью).
setInterval(poll, POLL_INTERVAL_MS);

// MV3-воркер может быть выгружен браузером между событиями; минимальный
// период у chrome.alarms — 1 минута, так что это подстраховка на случай,
// если setInterval прервался вместе с выгрузкой воркера, а не основной
// механизм (иначе задержка блокировки была бы до минуты).
chrome.alarms.create("netguardian-poll-fallback", { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name === "netguardian-poll-fallback") poll();
});

poll();

// Отдаёт popup.js закэшированное состояние (сам popup соединение с
// нативным хостом не держит, чтобы не плодить лишние процессы).
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg?.cmd === "get_cached_state") {
    sendResponse({ vpnConnected, blockedDomains });
  }
  return false;
});
