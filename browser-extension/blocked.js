const params = new URLSearchParams(location.search);
const site = params.get("site") || "—";
document.getElementById("siteName").textContent = site;

document.getElementById("retryBtn").addEventListener("click", () => {
  location.href = "https://" + site;
});
