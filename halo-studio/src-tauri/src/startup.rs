//! Iniciar com o Windows e retomar as luzes no boot.
//!
//! O autostart é um valor na chave Run do próprio usuário (HKCU), que não
//! precisa de administrador. O Windows abre o app com `--autostart`: ele fica
//! só na bandeja e, se as luzes estavam ligadas quando o PC desligou, liga de
//! novo, esperando o Arduino aparecer no USB.

use crate::{tray, AppState};
use serde::Serialize;
use std::time::{Duration, Instant};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

/// Argumento que o Windows passa quando abre o app no login.
pub const ARG: &str = "--autostart";
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("Halo Studio");
/// Quanto tempo esperar o Arduino aparecer no boot (o USB demora).
const RESUME_FOR: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
pub struct Autostart {
    pub enabled: bool,
    /// Linha registrada na chave Run.
    pub command: String,
    /// O registro aponta para este exe.
    pub current: bool,
    /// Rodando de dentro de `target/` (npm run dev): não registra.
    pub dev: bool,
}

fn command_line() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    Ok(format!("\"{}\" {ARG}", exe.display()))
}

/// O exe é o de desenvolvimento (`target\debug` ou `target\release`), não o instalado.
pub fn is_dev_build() -> bool {
    std::env::current_exe()
        .map(|p| p.components().any(|c| c.as_os_str().eq_ignore_ascii_case("target")))
        .unwrap_or(true)
}

fn registered() -> Option<String> {
    let mut len = 0u32;
    let first = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, RRF_RT_REG_SZ, None, None, Some(&mut len)) };
    if first != ERROR_SUCCESS || len == 0 {
        return None;
    }
    let mut buf = vec![0u16; len as usize / 2 + 1];
    let r = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if r != ERROR_SUCCESS {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..end]))
}

pub fn status() -> Autostart {
    let command = registered().unwrap_or_default();
    Autostart {
        enabled: !command.is_empty(),
        current: command_line().is_ok_and(|c| c.eq_ignore_ascii_case(&command)),
        command,
        dev: is_dev_build(),
    }
}

pub fn set(enabled: bool) -> Result<Autostart, String> {
    if enabled {
        if is_dev_build() {
            return Err("Esta é a versão de desenvolvimento (npm run dev). Instale o Halo Studio para ele iniciar com o Windows.".into());
        }
        let line = HSTRING::from(command_line()?);
        let bytes = ((line.len() + 1) * 2) as u32;
        let r = unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, REG_SZ.0, Some(line.as_ptr().cast()), bytes) };
        if r != ERROR_SUCCESS {
            return Err(format!("Não deu para gravar o início automático (erro {})", r.0));
        }
    } else {
        let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE) };
        if r != ERROR_SUCCESS && r != ERROR_FILE_NOT_FOUND {
            return Err(format!("Não deu para tirar o início automático (erro {})", r.0));
        }
    }
    Ok(status())
}

/// Na primeira vez que a versão instalada abre, liga o autostart (o usuário
/// pediu). Depois só mantém o registro apontando para o exe certo, se mudou
/// de lugar numa reinstalação.
pub fn first_run(state: &AppState) {
    if is_dev_build() {
        return;
    }
    let done = state.config.read().unwrap().state.autostart_done;
    let now = status();
    if !done {
        if set(true).is_ok() {
            let mut cfg = state.config.write().unwrap();
            cfg.state.autostart_done = true;
            let _ = cfg.save(&state.config_path);
        }
    } else if now.enabled && !now.current {
        let _ = set(true);
    }
}

/// Aberto pelo Windows: liga as luzes de novo se estavam ligadas, tentando
/// por até 30 s enquanto o Arduino não aparece no USB.
pub fn resume(state: AppState, app: tauri::AppHandle) {
    let (resume, was_running) = {
        let cfg = state.config.read().unwrap();
        (cfg.app.resume, cfg.state.was_running)
    };
    if !resume || !was_running {
        return;
    }
    let until = Instant::now() + RESUME_FOR;
    let last = loop {
        tray::note(&app, "Procurando o Arduino…");
        match crate::engine::start(state.clone()) {
            Ok(()) => {
                tray::note(&app, "");
                return;
            }
            Err(e) if Instant::now() >= until => break e,
            Err(_) => std::thread::sleep(Duration::from_secs(3)),
        }
    };
    tray::note(&app, &format!("Não deu para religar as luzes: {last}"));
    *state.engine.error.lock().unwrap() = Some(format!("Não deu para religar as luzes no boot: {last}"));
}
