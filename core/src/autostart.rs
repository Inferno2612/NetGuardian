//! Автозапуск при входе в систему через HKCU\...\Run — порт логики
//! из старого network_control.py (без части про отключение интернета).

#[cfg(windows)]
mod imp {
    use winreg::enums::*;
    use winreg::RegKey;

    const RUN_KEY_PATH: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const RUN_VALUE_NAME: &str = "NetGuardian";

    pub fn set_run_on_startup(enable: bool, exe_path: Option<&str>) -> anyhow::Result<()> {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (key, _) = hkcu.create_subkey(RUN_KEY_PATH)?;
        if enable {
            let path = match exe_path {
                Some(p) => p.to_string(),
                None => std::env::current_exe()?.display().to_string(),
            };
            key.set_value(RUN_VALUE_NAME, &path)?;
        } else {
            // Не ошибка, если значения и так не было.
            let _ = key.delete_value(RUN_VALUE_NAME);
        }
        Ok(())
    }

    pub fn is_run_on_startup_set() -> bool {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        match hkcu.open_subkey(RUN_KEY_PATH) {
            Ok(key) => key.get_value::<String, _>(RUN_VALUE_NAME).is_ok(),
            Err(_) => false,
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn set_run_on_startup(_enable: bool, _exe_path: Option<&str>) -> anyhow::Result<()> {
        Ok(())
    }
    pub fn is_run_on_startup_set() -> bool {
        false
    }
}

pub use imp::{is_run_on_startup_set, set_run_on_startup};
