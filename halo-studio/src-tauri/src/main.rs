#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod capture;
mod color;
mod config;
mod engine;
mod openrgb;
mod output;
mod startup;
mod tray;
mod visuals;
mod zones;

use config::Config;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;
use tauri::ipc::Response;
use tauri::{Manager, State, WindowEvent};

pub struct Shared {
    pub config: RwLock<Config>,
    /// Incrementa a cada mudança; o laço de quadros relê a configuração.
    pub config_version: AtomicU64,
    config_path: PathBuf,
    pub engine: engine::Engine,
    pub audio: audio::Monitor,
    /// Porta do Arduino aberta pelo assistente de mapeamento (só enquanto ele está aberto).
    pub mapper: Mutex<Option<Mapper>>,
}

/// Assistente de mapeamento aberto: a porta e onde a saída mapeada fica no quadro.
pub struct Mapper {
    port: output::Adalight,
    offset: usize,
    total: usize,
}

impl Mapper {
    fn blackout(&mut self) -> Result<(), String> {
        use output::LedOutput;
        self.port.send(&vec![openrgb::Color::default(); self.total.max(1)])
    }
}

/// Guarda se as luzes estão ligadas, para o próximo boot retomar igual.
fn remember_running(state: &Shared, on: bool) {
    let mut cfg = state.config.write().unwrap();
    if cfg.state.was_running != on {
        cfg.state.was_running = on;
        let _ = cfg.save(&state.config_path);
    }
}

type AppState = Arc<Shared>;

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.config.read().unwrap().clone()
}

#[tauri::command]
fn set_config(state: State<AppState>, mut config: Config) -> Result<(), String> {
    // `{}` (Restaurar padrões) chega sem versão e vira a configuração padrão.
    config.migrate();
    config.normalize();
    // O estado (luzes ligadas, autostart feito) é do backend; a interface manda uma cópia velha.
    config.state = state.config.read().unwrap().state.clone();
    *state.config.write().unwrap() = config.clone();
    state.config_version.fetch_add(1, Ordering::SeqCst);
    config.save(&state.config_path)
}

#[tauri::command]
fn list_monitors() -> Result<Vec<capture::MonitorInfo>, String> {
    capture::list_monitors()
}

#[tauri::command]
async fn list_ports() -> Result<Vec<output::PortInfo>, String> {
    Ok(output::list_ports())
}

#[tauri::command]
async fn start(state: State<'_, AppState>) -> Result<(), String> {
    let shared = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        engine::start(shared.clone())?;
        remember_running(&shared, true);
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn stop(state: State<AppState>) {
    state.engine.stop();
    state.audio.stop();
    remember_running(&state, false);
}

#[tauri::command]
fn get_autostart() -> startup::Autostart {
    startup::status()
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<startup::Autostart, String> {
    startup::set(enabled)
}

/// "Sair" de verdade (o X da janela só esconde na bandeja).
#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    tray::quit(&app);
}

#[derive(Serialize)]
struct Status {
    running: bool,
    error: Option<String>,
    config_path: String,
    simulate: bool,
    port: String,
    firmware: output::Firmware,
}

#[tauri::command]
fn status(state: State<AppState>) -> Status {
    let cfg = state.config.read().unwrap();
    Status {
        running: state.engine.is_running(),
        error: state.engine.error.lock().unwrap().clone(),
        config_path: state.config_path.display().to_string(),
        simulate: cfg.serial.simulate,
        port: cfg.serial.port.clone(),
        firmware: state.engine.firmware.lock().unwrap().clone(),
    }
}

/// A porta COM está livre para abrir agora? (O Halo rodando de verdade ou o
/// assistente aberto a seguram.)
fn port_free(state: &Shared, cfg: &Config, what: &str) -> Result<(), String> {
    if state.mapper.lock().unwrap().is_some() {
        return Err(format!("Feche o assistente de mapeamento antes de {what}"));
    }
    if state.engine.is_running() && !cfg.serial.simulate {
        return Err(format!("Pare o Halo antes de {what}: a porta COM está em uso por ele"));
    }
    Ok(())
}

/// Abre a porta COM só agora, configura as saídas e acende só a saída
/// `output` (vermelho, verde, azul e um ponto correndo). Depois fecha.
#[tauri::command]
async fn test_leds(state: State<'_, AppState>, output: usize) -> Result<output::Firmware, String> {
    let cfg = state.config.read().unwrap().clone();
    port_free(&state, &cfg, "testar")?;
    let start = *cfg.offsets().get(output).ok_or("Saída inexistente")?;
    let range = start..start + cfg.outputs[output].leds.count as usize;
    let layout = cfg.layout();
    let fw = tauri::async_runtime::spawn_blocking(move || {
        output::test_output(&cfg.serial.port, cfg.serial.baud, &layout, range)
    })
    .await
    .map_err(|e| e.to_string())??;
    state.engine.set_firmware(fw.clone());
    Ok(fw)
}

/// Manda a lista de saídas (pino, quantidade, ordem de cor) ao Arduino e
/// devolve a resposta. Usa a porta que já estiver aberta; senão abre e fecha.
#[tauri::command]
async fn apply_outputs(state: State<'_, AppState>) -> Result<output::Firmware, String> {
    use output::LedOutput;
    let cfg = state.config.read().unwrap().clone();
    let layout = cfg.layout();
    if state.engine.is_running() {
        // O laço manda no próximo quadro; espera a resposta chegar.
        let seq = state.engine.firmware_seq.load(std::sync::atomic::Ordering::SeqCst);
        state.engine.resync.store(true, std::sync::atomic::Ordering::SeqCst);
        let shared = state.inner().clone();
        return tauri::async_runtime::spawn_blocking(move || {
            for _ in 0..40 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if shared.engine.firmware_seq.load(std::sync::atomic::Ordering::SeqCst) != seq {
                    break;
                }
            }
            Ok(shared.engine.firmware.lock().unwrap().clone())
        })
        .await
        .map_err(|e| e.to_string())?;
    }
    if let Some(m) = state.mapper.lock().unwrap().as_mut() {
        let fw = m.port.configure(&layout);
        state.engine.set_firmware(fw.clone());
        return Ok(fw);
    }
    let fw = if cfg.serial.simulate {
        output::SerialLog::new().configure(&layout)
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            output::Adalight::open(&cfg.serial.port, cfg.serial.baud).map(|mut p| p.configure(&layout))
        })
        .await
        .map_err(|e| e.to_string())??
    };
    state.engine.set_firmware(fw.clone());
    Ok(fw)
}

/// Um quadro do efeito da saída `output`, calculado só para a prévia da tela
/// Efeitos com o Halo parado. Não envia nada aos LEDs. RGB por LED, com o brilho.
#[tauri::command]
fn effect_preview(state: State<AppState>, output: usize) -> Response {
    static START: OnceLock<Instant> = OnceLock::new();
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f32();
    let cfg = state.config.read().unwrap().clone();
    let Some(o) = cfg.outputs.get(output) else {
        return Response::new(Vec::<u8>::new());
    };
    let pos = visuals::positions(&o.leds);
    let mut strip = vec![[0.0; 3]; pos.len()];
    visuals::render_effect(&o.effect, t, &pos, &mut strip);
    let (mut out, mut display) = (Vec::new(), Vec::new());
    color::Adjust::for_output(&cfg.color, o).apply(&strip, &mut out, &mut display);
    Response::new(display.iter().flat_map(|c| [c.r, c.g, c.b]).collect::<Vec<u8>>())
}

/// Assistente de mapeamento da saída `output`: abre a porta do Arduino,
/// configura as saídas e mantém a porta aberta até `mapper_close`, para
/// acender um pixel por vez sem reiniciar o Arduino.
#[tauri::command]
async fn mapper_open(state: State<'_, AppState>, output: usize) -> Result<output::Firmware, String> {
    use output::LedOutput;
    let cfg = state.config.read().unwrap().clone();
    let offset = *cfg.offsets().get(output).ok_or("Saída inexistente")?;
    if let Some(m) = state.mapper.lock().unwrap().as_mut() {
        m.offset = offset;
        return Ok(state.engine.firmware.lock().unwrap().clone());
    }
    port_free(&state, &cfg, "mapear")?;
    let layout = cfg.layout();
    let total = cfg.total_leds();
    let (port, fw) = tauri::async_runtime::spawn_blocking(move || {
        output::Adalight::open(&cfg.serial.port, cfg.serial.baud).map(|mut p| {
            let fw = p.configure(&layout);
            (p, fw)
        })
    })
    .await
    .map_err(|e| e.to_string())??;
    state.engine.set_firmware(fw.clone());
    *state.mapper.lock().unwrap() = Some(Mapper { port, offset, total });
    Ok(fw)
}

/// Manda as cores dos pixels da saída mapeada; as outras saídas ficam apagadas.
#[tauri::command]
fn mapper_send(state: State<AppState>, colors: Vec<openrgb::Color>) -> Result<(), String> {
    use output::LedOutput;
    let mut guard = state.mapper.lock().unwrap();
    let m = guard.as_mut().ok_or("O assistente de mapeamento não está com a porta aberta")?;
    let mut frame = vec![openrgb::Color::default(); m.total.max(m.offset + colors.len())];
    frame[m.offset..m.offset + colors.len()].copy_from_slice(&colors);
    m.port.send(&frame).inspect_err(|_| *guard = None)
}

/// Apaga os pixels e solta a porta COM.
#[tauri::command]
fn mapper_close(state: State<AppState>) {
    if let Some(mut m) = state.mapper.lock().unwrap().take() {
        let _ = m.blackout();
    }
}

#[derive(Serialize)]
struct RgbDevice {
    index: u32,
    name: String,
    vendor: String,
    kind: i32,
    leds: usize,
    streamable: bool,
}

/// Só lê a lista de dispositivos do OpenRGB; não muda nada neles.
#[tauri::command]
async fn openrgb_devices(host: String, port: u16) -> Result<Vec<RgbDevice>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut c = openrgb::Client::connect(&host, port, "Halo Studio")
            .map_err(|e| format!("OpenRGB não respondeu em {host}:{port}: {e}"))?;
        let devices = c.devices().map_err(|e| format!("OpenRGB: {e}"))?;
        Ok(devices
            .iter()
            .map(|d| RgbDevice {
                index: d.index,
                name: d.name.clone(),
                vendor: d.vendor.clone(),
                kind: d.kind,
                leds: d.leds.len(),
                streamable: output::can_stream(d),
            })
            .collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Quadro atual em binário (little-endian), para a prévia não pagar JSON:
///
/// ```text
/// u32 seq | u16 img_w | u16 img_h | u16 zonas | u16 leds | u16 faixas | u16 flags
/// f32 fps | f32 fps_captura | f32 latência_ms | f32 corrente_mA | f32×4 área útil
/// f32×4 por zona (retângulo) | RGBA img_w×img_h | RGB por zona | RGB por LED | f32 por faixa
/// f32 por saída (corrente em mA)
/// ```
///
/// As zonas são as da saída `output` (uma fita); os LEDs são de todas as
/// saídas em sequência.
#[tauri::command]
fn get_frame(state: State<AppState>, with_image: bool, output: Option<usize>) -> Response {
    let zones_cfg = output.and_then(|i| state.config.read().unwrap().outputs.get(i).map(|o| o.zones.clone()));
    let running = state.engine.is_running();
    let f = state.engine.frame.lock().unwrap();
    let (w, h) = if with_image { (f.image.width, f.image.height) } else { (0, 0) };
    let aspect = if f.image.height > 0 { f.image.width as f32 / f.image.height as f32 } else { 16.0 / 9.0 };
    let rects = zones_cfg.map(|z| zones::layout(&z, aspect)).unwrap_or_default();
    let zone_count = rects.len();
    let zone_colors = output.and_then(|i| f.zones.get(i));

    let mut b = Vec::with_capacity(64 + zone_count * 19 + (w * h * 4) as usize + f.leds.len() * 3);
    b.extend(f.seq.to_le_bytes());
    // flags: bit 0 = rodando, bits 8.. = quantas correntes vêm no fim.
    let flags = running as u32 | (f.currents.len() as u32) << 8;
    for v in [w, h, zone_count as u32, f.leds.len() as u32, f.bands.len() as u32, flags] {
        b.extend((v as u16).to_le_bytes());
    }
    for v in [f.fps, f.capture_fps, f.latency_ms, f.current_ma] {
        b.extend(v.to_le_bytes());
    }
    let content = if f.image.width > 0 { f.content } else { [0.0, 0.0, 1.0, 1.0] };
    for v in content.iter().chain(rects.iter().flatten()) {
        b.extend(v.to_le_bytes());
    }
    if w > 0 {
        for p in f.image.bgra.chunks_exact(4) {
            b.extend([p[2], p[1], p[0], 255]);
        }
    }
    for i in 0..zone_count {
        let c = zone_colors.and_then(|z| z.get(i)).copied().unwrap_or_default();
        b.extend([c.r, c.g, c.b]);
    }
    for c in &f.leds {
        b.extend([c.r, c.g, c.b]);
    }
    for v in &f.bands {
        b.extend(v.to_le_bytes());
    }
    for v in &f.currents {
        b.extend(v.to_le_bytes());
    }
    Response::new(b)
}

fn main() {
    let autostart = std::env::args().any(|a| a == startup::ARG);
    tauri::Builder::default()
        // Abrir de novo (Menu Iniciar) só mostra a janela que já está na bandeja:
        // dois Halos brigariam pela porta COM.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show(app)))
        .on_window_event(|window, event| {
            // O X esconde na bandeja; "Sair" fica no menu do ícone.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(move |app| {
            let path = app
                .path()
                .app_config_dir()
                .map(|d| d.join("config.json"))
                .unwrap_or_else(|_| PathBuf::from("halo-studio.json"));
            let state: AppState = Arc::new(Shared {
                config: RwLock::new(Config::load(&path)),
                config_version: AtomicU64::new(0),
                config_path: path,
                engine: engine::Engine::default(),
                audio: audio::Monitor::default(),
                mapper: Mutex::new(None),
            });
            app.manage(state.clone());
            tray::build(app)?;
            startup::first_run(&state);
            if autostart {
                // Aberto pelo Windows: só a bandeja, e religa as luzes se estavam ligadas.
                let handle = app.handle().clone();
                std::thread::spawn(move || startup::resume(state, handle));
            } else {
                tray::show(app.handle());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            set_config,
            list_monitors,
            list_ports,
            start,
            stop,
            status,
            test_leds,
            apply_outputs,
            openrgb_devices,
            effect_preview,
            mapper_open,
            mapper_send,
            mapper_close,
            get_frame,
            get_autostart,
            set_autostart,
            quit_app,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o Halo Studio");
}
