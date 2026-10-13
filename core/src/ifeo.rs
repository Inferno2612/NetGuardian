//! Надёжный режим блокировки приложений через Image File Execution Options (IFEO).
//!
//! Как это работает: в HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\
//! Image File Execution Options\<имя.exe> создаётся значение "Debugger",
//! указывающее на сам NetGuardian.exe. После этого Windows при ЛЮБОЙ попытке
//! запустить <имя.exe> (из ярлыка, из меню Пуск, из другой программы —
//! откуда угодно) вместо него на самом деле запускает:
//!
//!     NetGuardian.exe --ifeo-stub "<полный путь к исходному .exe>" <оригинальные аргументы>
//!
//! main.rs перехватывает этот режим запуска в самом начале (до создания
//! обычного окна), проверяет статус VPN и либо прозрачно запускает
//! оригинальное приложение (VPN включён), либо показывает окно-уведомление
//! и оригинальное приложение НЕ запускает (VPN выключен).
//!
//! Требует прав администратора на запись в HKLM — см. elevation.rs.
//! Это более надёжный, но и более инвазивный способ, чем простой
//! процесс-вотчер (app_blocking.rs), который ничего не пишет в реестр,
//! но может не успеть убить приложение за первую долю секунды после запуска.

#[cfg(windows)]
mod imp {
    use winreg::enums::*;
    use winreg::RegKey;

    const IFEO_BASE: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Image File Execution Options";
    const DEBUGGER_VALUE: &str = "Debugger";
    /// Своё значение-маркер, чтобы при включении/выключении точно узнавать,
    /// что Debugger поставлен именно нами, а не сторонним отладчиком/AV.
    const MARKER_VALUE: &str = "NetGuardianManaged";

    fn normalize_exe_name(name: &str) -> String {
        let n = name.trim().to_lowercase();
        if n.ends_with(".exe") {
            n
        } else {
            format!("{n}.exe")
        }
    }

    /// Включает IFEO-перехват для указанного имени exe (например, "telegram.exe").
    /// Требует прав администратора — вызывающий код должен сам проверить is_admin().
    pub fn enable_guard(exe_name: &str) -> anyhow::Result<()> {
        let exe_name = normalize_exe_name(exe_name);
        let our_exe = std::env::current_exe()?;
        let our_exe_str = our_exe.display().to_string();

        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let (key, _) = hklm.create_subkey(format!(r"{IFEO_BASE}\{exe_name}"))?;

        // Debugger-значение: наш .exe плюс флаг "--ifeo-stub" — исходный
        // путь и аргументы Windows допишет сама при реальном запуске.
        let debugger_cmd = format!("\"{our_exe_str}\" --ifeo-stub");
        key.set_value(DEBUGGER_VALUE, &debugger_cmd)?;
        key.set_value(MARKER_VALUE, &"1".to_string())?;
        Ok(())
    }

    /// Снимает IFEO-перехват, но только если он был поставлен нами
    /// (проверяется по MARKER_VALUE) — чтобы случайно не удалить
    /// чужую запись Debugger, если она там уже была.
    pub fn disable_guard(exe_name: &str) -> anyhow::Result<()> {
        let exe_name = normalize_exe_name(exe_name);
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let subkey_path = format!(r"{IFEO_BASE}\{exe_name}");

        let key = match hklm.open_subkey_with_flags(&subkey_path, KEY_ALL_ACCESS) {
            Ok(k) => k,
            Err(_) => return Ok(()), // ключа и так нет — нечего снимать
        };

        let is_ours: bool = key
            .get_value::<String, _>(MARKER_VALUE)
            .map(|v| v == "1")
            .unwrap_or(false);

        if !is_ours {
            anyhow::bail!(
                "Запись Image File Execution Options для {exe_name} существует, но поставлена не NetGuardian — не трогаю её из соображений безопасности."
            );
        }

        let _ = key.delete_value(DEBUGGER_VALUE);
        let _ = key.delete_value(MARKER_VALUE);
        drop(key);

        // Если в подразделе больше ничего не осталось — удаляем и его целиком.
        if let Ok(k) = hklm.open_subkey(&subkey_path) {
            if k.enum_values().next().is_none() {
                let _ = hklm.delete_subkey(&subkey_path);
            }
        }
        Ok(())
    }

    /// Временно «усыпляет» перехват: убирает значение Debugger, но оставляет
    /// ключ и маркер. Нужен, пока VPN включён: многопроцессные приложения
    /// (Electron/Chromium — Discord, Claude и т.п.) запускают дочерние
    /// процессы со специальным наследованием дескрипторов, и перезапуск
    /// каждого через заглушку их ломает. Возвращает true, если что-то изменилось.
    pub fn suspend_guard(exe_name: &str) -> anyhow::Result<bool> {
        let exe_name = normalize_exe_name(exe_name);
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = match hklm.open_subkey_with_flags(
            format!(r"{IFEO_BASE}\{exe_name}"),
            KEY_ALL_ACCESS,
        ) {
            Ok(k) => k,
            Err(_) => return Ok(false),
        };
        let is_ours = key
            .get_value::<String, _>(MARKER_VALUE)
            .map(|v| v == "1")
            .unwrap_or(false);
        if !is_ours {
            return Ok(false);
        }
        if key.get_value::<String, _>(DEBUGGER_VALUE).is_err() {
            return Ok(false); // уже усыплён
        }
        key.delete_value(DEBUGGER_VALUE)?;
        Ok(true)
    }

    /// Возвращает перехват после suspend_guard. true, если что-то изменилось.
    pub fn resume_guard(exe_name: &str) -> anyhow::Result<bool> {
        let exe_name = normalize_exe_name(exe_name);
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = match hklm.open_subkey_with_flags(
            format!(r"{IFEO_BASE}\{exe_name}"),
            KEY_ALL_ACCESS,
        ) {
            Ok(k) => k,
            Err(_) => return Ok(false),
        };
        let is_ours = key
            .get_value::<String, _>(MARKER_VALUE)
            .map(|v| v == "1")
            .unwrap_or(false);
        if !is_ours || key.get_value::<String, _>(DEBUGGER_VALUE).is_ok() {
            return Ok(false);
        }
        let our_exe = std::env::current_exe()?;
        let cmd = format!("\"{}\" --ifeo-stub", our_exe.display());
        key.set_value(DEBUGGER_VALUE, &cmd)?;
        Ok(true)
    }

    pub fn is_guard_enabled(exe_name: &str) -> bool {
        let exe_name = normalize_exe_name(exe_name);
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        match hklm.open_subkey(format!(r"{IFEO_BASE}\{exe_name}")) {
            Ok(key) => key
                .get_value::<String, _>(MARKER_VALUE)
                .map(|v| v == "1")
                .unwrap_or(false),
            Err(_) => false,
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn enable_guard(_exe_name: &str) -> anyhow::Result<()> {
        anyhow::bail!("Доступно только на Windows")
    }
    pub fn disable_guard(_exe_name: &str) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn is_guard_enabled(_exe_name: &str) -> bool {
        false
    }
    pub fn suspend_guard(_exe_name: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
    pub fn resume_guard(_exe_name: &str) -> anyhow::Result<bool> {
        Ok(false)
    }
}

pub use imp::{disable_guard, enable_guard, is_guard_enabled, resume_guard, suspend_guard};
