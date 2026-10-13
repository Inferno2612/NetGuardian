<#
.SYNOPSIS
  Регистрирует нативный хост NetGuardian (netguardian_native_host.exe) для
  Native Messaging в Chrome, Edge и Firefox, чтобы расширение браузера
  могло спрашивать статус VPN у десктоп-приложения NetGuardian.

.DESCRIPTION
  Запускать ОДИН РАЗ после того, как:
    1. Собран native-host (cargo build --release -p netguardian-native-host),
       .exe лежит в src-tauri\..\target\release\netguardian_native_host.exe
       (или укажите свой путь через -ExePath).
    2. Расширение загружено в браузер как распакованное (Chrome/Edge:
       chrome://extensions → "Режим разработчика" → "Загрузить
       распакованное расширение" → папка browser-extension) — после этого
       браузер покажет ID расширения, его нужно передать в -ChromeExtensionId
       (для Edge обычно совпадает, но проверьте в edge://extensions).

.PARAMETER ExePath
  Путь к собранному netguardian_native_host.exe. По умолчанию ищет рядом
  со скриптом в target\release\.

.PARAMETER ChromeExtensionId
  ID расширения в Chrome (32 латинские буквы), с chrome://extensions.

.PARAMETER EdgeExtensionId
  ID расширения в Edge, с edge://extensions. Если не указан — используется
  тот же, что и ChromeExtensionId (обычно совпадает для распакованных
  расширений с одинаковым путём).

.EXAMPLE
  .\install-native-host.ps1 -ChromeExtensionId "abcdefghijklmnopabcdefghijklmnop"
#>

param(
    [string]$ExePath,
    [string]$ChromeExtensionId,
    [string]$EdgeExtensionId
)

$ErrorActionPreference = "Stop"

$HostName = "ru.netguardian.nativehost"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path

if (-not $ExePath) {
    $candidate = Join-Path $ScriptDir "target\release\netguardian_native_host.exe"
    if (Test-Path $candidate) {
        $ExePath = $candidate
    } else {
        throw "Не найден netguardian_native_host.exe. Соберите его (cargo build --release -p netguardian-native-host) или укажите путь через -ExePath."
    }
}
$ExePath = (Resolve-Path $ExePath).Path

if (-not $ChromeExtensionId -and -not $EdgeExtensionId) {
    Write-Warning "Не указан ни -ChromeExtensionId, ни -EdgeExtensionId — манифест для Chrome/Edge будет пропущен. Загрузите расширение в браузер, скопируйте его ID со страницы расширений и запустите скрипт снова с этим параметром."
}
if (-not $EdgeExtensionId) { $EdgeExtensionId = $ChromeExtensionId }

$ManifestDir = Join-Path $env:LOCALAPPDATA "NetGuardian\native-messaging"
New-Item -ItemType Directory -Force -Path $ManifestDir | Out-Null

function Write-HostManifest {
    param([string]$Path, [hashtable]$Data)
    $json = $Data | ConvertTo-Json -Depth 5
    Set-Content -Path $Path -Value $json -Encoding UTF8
    Write-Host "Записан манифест: $Path"
}

function Set-NativeMessagingRegistryKey {
    param([string]$BrowserRegPath, [string]$ManifestPath)
    New-Item -Path $BrowserRegPath -Force | Out-Null
    Set-ItemProperty -Path $BrowserRegPath -Name "(Default)" -Value $ManifestPath
    Write-Host "Зарегистрирован реестр: $BrowserRegPath -> $ManifestPath"
}

# ---------- Chrome / Edge (используют одинаковый формат: allowed_origins) ----------

if ($ChromeExtensionId -or $EdgeExtensionId) {
    $chromiumManifestPath = Join-Path $ManifestDir "chromium-host-manifest.json"

    $allowedOrigins = @()
    if ($ChromeExtensionId) { $allowedOrigins += "chrome-extension://$ChromeExtensionId/" }
    if ($EdgeExtensionId -and $EdgeExtensionId -ne $ChromeExtensionId) {
        $allowedOrigins += "chrome-extension://$EdgeExtensionId/"
    }

    Write-HostManifest -Path $chromiumManifestPath -Data @{
        name            = $HostName
        description     = "NetGuardian — статус VPN для блокировки сайтов"
        path            = $ExePath
        type            = "stdio"
        allowed_origins = $allowedOrigins
    }

    if ($ChromeExtensionId) {
        Set-NativeMessagingRegistryKey `
            -BrowserRegPath "HKCU:\Software\Google\Chrome\NativeMessagingHosts\$HostName" `
            -ManifestPath $chromiumManifestPath
    }
    if ($EdgeExtensionId) {
        Set-NativeMessagingRegistryKey `
            -BrowserRegPath "HKCU:\Software\Microsoft\Edge\NativeMessagingHosts\$HostName" `
            -ManifestPath $chromiumManifestPath
    }
}

# ---------- Firefox (формат отличается: allowed_extensions + id из manifest.firefox.json) ----------

$firefoxManifestPath = Join-Path $ManifestDir "firefox-host-manifest.json"
Write-HostManifest -Path $firefoxManifestPath -Data @{
    name              = $HostName
    description       = "NetGuardian — статус VPN для блокировки сайтов"
    path              = $ExePath
    type              = "stdio"
    allowed_extensions = @("netguardian@localhost")
}
Set-NativeMessagingRegistryKey `
    -BrowserRegPath "HKCU:\Software\Mozilla\NativeMessagingHosts\$HostName" `
    -ManifestPath $firefoxManifestPath

Write-Host ""
Write-Host "Готово. Перезапустите браузер(ы), чтобы расширение подхватило нативный хост."
