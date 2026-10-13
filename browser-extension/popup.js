chrome.runtime.sendMessage({ cmd: "get_cached_state" }, (resp) => {
  const dot = document.getElementById("dot");
  const text = document.getElementById("statusText");
  const domainsText = document.getElementById("domainsText");

  if (!resp) {
    text.textContent = "Нет связи с расширением";
    return;
  }

  if (resp.vpnConnected === true) {
    dot.classList.add("on");
    text.textContent = "VPN включён";
  } else if (resp.vpnConnected === false) {
    dot.classList.add("off");
    text.textContent = "VPN выключен — блокировка активна";
  } else {
    text.textContent = "NetGuardian не отвечает";
  }

  const n = (resp.blockedDomains || []).length;
  domainsText.textContent = n > 0 ? `Заблокированных сайтов: ${n}` : "Список сайтов пуст";
});
