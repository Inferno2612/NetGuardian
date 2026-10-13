# Расширение NetGuardian для браузера

Блокирует переход на сайты из списка (вкладка «Блокировка» в
NetGuardian), пока VPN выключен. Связывается с десктоп-приложением
через нативный хост (`native-host/`), а не напрямую — так что статус
VPN в браузере всегда совпадает с тем, что видит само приложение.

## Установка (один раз)

### 1. Соберите нативный хост

```powershell
cd C:\projects\WI-Fi
cargo build --release -p netguardian-native-host
```

Появится `target\release\netguardian_native_host.exe`.

### 2. Загрузите расширение в браузер

**Chrome / Edge:**
1. Откройте `chrome://extensions` (или `edge://extensions`).
2. Включите «Режим разработчика» (переключатель справа вверху).
3. «Загрузить распакованное расширение» → выберите папку
   `browser-extension`.
4. Скопируйте появившийся **ID расширения** (32 латинские буквы) —
   он понадобится на следующем шаге.

**Firefox:**
1. Перед загрузкой скопируйте `manifest.firefox.json` поверх
   `manifest.json` в папке `browser-extension` (Firefox использует
   другой формат описания фонового скрипта).
2. Откройте `about:debugging#/runtime/this-firefox` → «Загрузить
   временное дополнение» → выберите файл `manifest.json` в папке
   `browser-extension`.
3. Такое временное дополнение исчезает при перезапуске Firefox — чтобы
   поставить постоянно, расширение нужно подписать через
   addons.mozilla.org (или использовать Firefox Developer/Nightly с
   отключённой проверкой подписи). Это ограничение самого Firefox, не
   NetGuardian.

### 3. Зарегистрируйте нативный хост

Из PowerShell, из папки проекта:

```powershell
.\install-native-host.ps1 -ChromeExtensionId "ваш_id_из_chrome_extensions"
```

Если используете и Chrome, и Edge, и ID расширения в них разный —
добавьте `-EdgeExtensionId "..."`. Для Firefox скрипт регистрирует хост
автоматически (использует фиксированный id `netguardian@localhost` из
`manifest.firefox.json`), отдельно указывать ничего не нужно.

### 4. Перезапустите браузер(ы)

После перезапуска значок NetGuardian в панели расширений должен
показывать статус VPN (кликните на него).

## Диагностика

- Если попап показывает «NetGuardian не отвечает» — проверьте, что
  `netguardian_native_host.exe` собран и путь в
  `%LOCALAPPDATA%\NetGuardian\native-messaging\chromium-host-manifest.json`
  (или `firefox-host-manifest.json`) указывает на реальный файл.
- Откройте консоль сервис-воркера расширения
  (`chrome://extensions` → NetGuardian → «Service worker» → «проверить»)
  — там будут видны ошибки подключения к нативному хосту.
- Эта часть проекта не тестировалась в реальном браузере (разработка
  велась без доступа к Windows/браузеру) — если что-то не заработает с
  первого раза, пришлите текст ошибки из консоли сервис-воркера, поправлю.
