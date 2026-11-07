//! Ícone na bandeja (perto do relógio): abrir, pausar/retomar as luzes e
//! sair. Fechar a janela só esconde; "Sair" encerra e apaga os LEDs.

use crate::{engine, remember_running, AppState};
use std::sync::Mutex;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

pub struct Tray {
    icon: TrayIcon<Wry>,
    status: MenuItem<Wry>,
    toggle: MenuItem<Wry>,
    /// Aviso que aparece no lugar do estado (ex.: "Procurando o Arduino…").
    note: Mutex<String>,
    /// Último texto mostrado, para não refazer o menu à toa.
    shown: Mutex<(String, bool)>,
}

pub fn build(app: &tauri::App) -> tauri::Result<()> {
    let status = MenuItem::with_id(app, "status", "Luzes paradas", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Abrir o Halo Studio", true, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", "Ligar as luzes", true, None::<&str>)?;
    let exit = MenuItem::with_id(app, "quit", "Sair (apaga os LEDs)", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&status, &PredefinedMenuItem::separator(app)?, &open, &toggle, &PredefinedMenuItem::separator(app)?, &exit],
    )?;
    let mut builder = TrayIconBuilder::with_id("halo")
        .tooltip("Halo Studio")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show(app),
            "toggle" => toggle_lights(app),
            "quit" => quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let icon = builder.build(app)?;
    app.manage(Tray { icon, status, toggle, note: Mutex::new(String::new()), shown: Mutex::default() });

    // Mantém o texto do menu e da dica em dia com o estado das luzes.
    let handle = app.handle().clone();
    std::thread::spawn(move || loop {
        refresh(&handle);
        std::thread::sleep(Duration::from_secs(1));
    });
    Ok(())
}

/// Mostra um aviso na bandeja no lugar do estado (vazio = volta ao normal).
pub fn note(app: &AppHandle, text: &str) {
    if let Some(t) = app.try_state::<Tray>() {
        *t.note.lock().unwrap() = text.to_string();
    }
    refresh(app);
}

fn refresh(app: &AppHandle) {
    let (Some(t), Some(state)) = (app.try_state::<Tray>(), app.try_state::<AppState>()) else {
        return;
    };
    let running = state.engine.is_running();
    let note = t.note.lock().unwrap().clone();
    let text = if running {
        "Luzes ligadas".to_string()
    } else if !note.is_empty() {
        note
    } else {
        "Luzes pausadas".to_string()
    };
    let mut shown = t.shown.lock().unwrap();
    if *shown == (text.clone(), running) {
        return;
    }
    let _ = t.status.set_text(&text);
    let _ = t.toggle.set_text(if running { "Pausar as luzes" } else { "Ligar as luzes" });
    // A dica do Windows aceita no máximo ~127 caracteres.
    let tip: String = format!("Halo Studio — {text}").chars().take(120).collect();
    let _ = t.icon.set_tooltip(Some(tip));
    *shown = (text, running);
}

pub fn show(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn toggle_lights(app: &AppHandle) {
    let state = app.state::<AppState>().inner().clone();
    if state.engine.is_running() {
        state.engine.stop();
        state.audio.stop();
        remember_running(&state, false);
        refresh(app);
        return;
    }
    note(app, "Ligando as luzes…");
    let app = app.clone();
    // Abrir a porta espera o Arduino reiniciar (~2 s): fora da thread da interface.
    std::thread::spawn(move || match engine::start(state.clone()) {
        Ok(()) => {
            remember_running(&state, true);
            note(&app, "");
        }
        Err(e) => {
            note(&app, &format!("Não ligou: {e}"));
            *state.engine.error.lock().unwrap() = Some(e);
        }
    });
}

/// Sai de verdade: para o laço (que manda o quadro preto) e encerra. Não mexe
/// no "estava ligado", para o próximo boot retomar do mesmo jeito.
pub fn quit(app: &AppHandle) {
    let state = app.state::<AppState>().inner().clone();
    let was = state.engine.is_running();
    state.engine.stop();
    state.audio.stop();
    if was {
        // O laço termina o quadro atual (até ~70 ms com o ACK) e apaga os LEDs.
        std::thread::sleep(Duration::from_millis(300));
    }
    if let Some(m) = state.mapper.lock().unwrap().as_mut() {
        let _ = m.blackout();
    }
    app.exit(0);
}
