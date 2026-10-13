//! Проверка/запрос прав администратора — Rust-порт is_admin()/relaunch_as_admin()
//! из старого main.py. Нужно только для включения надёжного режима блокировки
//! приложений через IFEO (реестр HKLM), который требует прав администратора.
//! Обычный процесс-вотчер (app_blocking.rs) прав администратора не требует.

#[cfg(windows)]
mod imp {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    pub fn is_admin() -> bool {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Security::{
            GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
        };
        use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        unsafe {
            let mut token = HANDLE::default();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION::default();
            let mut ret_len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            );
            ok.is_ok() && elevation.TokenIsElevated != 0
        }
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Перезапускает текущий .exe с запросом UAC (runas) и теми же аргументами.
    /// Возвращает Ok(()) если UAC-диалог успешно показан и подтверждён,
    /// иначе — текст ошибки на русском для показа пользователю.
    pub fn relaunch_as_admin(extra_args: &[String]) -> Result<(), String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe_wide = to_wide(&exe.display().to_string());

        let params = extra_args
            .iter()
            .map(|a| format!("\"{a}\""))
            .collect::<Vec<_>>()
            .join(" ");
        let params_wide = to_wide(&params);
        let verb_wide = to_wide("runas");

        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb_wide.as_ptr()),
                PCWSTR(exe_wide.as_ptr()),
                PCWSTR(params_wide.as_ptr()),
                None,
                SW_SHOWNORMAL,
            )
        };

        // ShellExecuteW возвращает HINSTANCE; значение <= 32 означает ошибку.
        let code = result.0 as isize;
        if code <= 32 {
            let msg = match code {
                0 => "Системе не хватило памяти или ресурсов.",
                2 => "Файл не найден.",
                3 => "Путь не найден.",
                5 => "Доступ запрещён (запрос повышения прав отклонён).",
                8 => "Не хватило памяти.",
                26 => "Общий сбой доступа к файлу.",
                31 => "Не найдено приложение, связанное с файлом.",
                _ => "Неизвестная ошибка ShellExecuteW.",
            };
            return Err(format!("{msg} (код {code})"));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn is_admin() -> bool {
        false
    }
    pub fn relaunch_as_admin(_extra_args: &[String]) -> Result<(), String> {
        Err("Доступно только на Windows".to_string())
    }
}

pub use imp::{is_admin, relaunch_as_admin};
