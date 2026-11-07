//! Laço principal: a cada quadro cada saída calcula as próprias cores pelo
//! modo dela (tela, cor da tela, música, efeito), aplica o brilho e o limite
//! de consumo dela e o global, e o quadro com todas as saídas em sequência
//! vai para o Arduino.

use crate::audio::{self, Analyzer};
use crate::capture::{self, Duplicator, Image};
use crate::color::Adjust;
use crate::config::{CaptureCfg, ColorCfg, Config, OutputCfg, OutputMode, SoundBand};
use crate::openrgb::Color;
use crate::output::{self, Firmware, LedOutput};
use crate::visuals::{self, AudioState};
use crate::zones;
use crate::Shared;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Último quadro calculado, lido pela interface para a prévia.
#[derive(Default)]
pub struct Frame {
    pub seq: u32,
    pub image: Image,
    /// Área útil (sem barra de tarefas / faixas pretas), normalizada.
    pub content: [f32; 4],
    /// Cor de cada zona, por saída (vazio nas saídas que não estão em Tela).
    pub zones: Vec<Vec<Color>>,
    /// Todas as saídas em sequência, como vão para o Arduino (cores sem gama).
    pub leds: Vec<Color>,
    pub bands: Vec<f32>,
    pub fps: f32,
    pub capture_fps: f32,
    pub latency_ms: f32,
    /// Corrente estimada: total e de cada saída (mA).
    pub current_ma: f32,
    pub currents: Vec<f32>,
}

#[derive(Default)]
pub struct Engine {
    /// Cada `start`/`stop` incrementa; a thread antiga percebe e termina.
    generation: AtomicU64,
    running: AtomicBool,
    pub error: Mutex<Option<String>>,
    pub frame: Mutex<Frame>,
    /// Última resposta do Arduino sobre as saídas.
    pub firmware: Mutex<Firmware>,
    /// Incrementa a cada resposta nova em `firmware`.
    pub firmware_seq: AtomicU64,
    /// Pede ao laço para reenviar a configuração das saídas (HloC).
    pub resync: AtomicBool,
}

impl Engine {
    pub fn stop(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn set_firmware(&self, fw: Firmware) {
        *self.firmware.lock().unwrap() = fw;
        self.firmware_seq.fetch_add(1, Ordering::SeqCst);
    }

    fn fail(&self, gen: u64, msg: String) {
        if self.generation.load(Ordering::SeqCst) == gen {
            *self.error.lock().unwrap() = Some(msg);
            self.running.store(false, Ordering::SeqCst);
        }
    }
}

/// Abre as saídas, configura os pinos no Arduino e começa o laço numa thread
/// própria. Bloqueia enquanto o Arduino reinicia (~2 s), então chame fora da
/// thread da interface.
pub fn start(shared: Arc<Shared>) -> Result<(), String> {
    if shared.mapper.lock().unwrap().is_some() {
        return Err("Feche o assistente de mapeamento antes de iniciar: ele está com a porta COM".into());
    }
    shared.engine.stop();
    shared.audio.stop();
    // Dá tempo para a thread anterior soltar a porta COM e mandar o quadro preto.
    std::thread::sleep(Duration::from_millis(120));
    let mut cfg = shared.config.read().unwrap().clone();
    if !cfg.serial.simulate {
        // O Arduino pode ter trocado de USB (COM3 → COM4): acha pela marca do chip.
        if let Some(port) = output::find_arduino(&cfg.serial.port) {
            if port != cfg.serial.port {
                let mut live = shared.config.write().unwrap();
                live.serial.port = port.clone();
                let _ = live.save(&shared.config_path);
                cfg.serial.port = port;
            }
        }
    }
    if cfg.outputs.is_empty() {
        return Err("Adicione uma saída em Dispositivos antes de iniciar".into());
    }
    if cfg.outputs.iter().any(|o| o.mode.uses_screen()) {
        // Falha cedo (ex.: monitor inexistente) em vez de só no laço.
        drop(Duplicator::new(cfg.capture.monitor)?);
    }
    let mut outputs = output::open_all(&cfg)?;
    shared.engine.set_firmware(outputs[0].configure(&cfg.layout()));
    let gen = shared.engine.generation.fetch_add(1, Ordering::SeqCst) + 1;
    *shared.engine.error.lock().unwrap() = None;
    shared.engine.running.store(true, Ordering::SeqCst);
    std::thread::Builder::new()
        .name("halo-engine".into())
        .spawn(move || run(shared, gen, outputs))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Estado de uma saída entre quadros.
#[derive(Default)]
struct OutState {
    cfg: OutputCfg,
    rects: Vec<zones::Rect>,
    map: Vec<Option<usize>>,
    /// Posição de cada LED para efeitos e música (ver `visuals::positions`).
    pos: Vec<visuals::Pos>,
    adjust: Option<Adjust>,
    /// Tela por zonas: cor medida e cor suavizada de cada zona.
    targets: Vec<[f32; 3]>,
    smooth: Vec<[f32; 3]>,
    /// Cor da tela: dominante medida e suavizada.
    dom_target: [f32; 3],
    dom: [f32; 3],
    music: AudioState,
    /// "Reagir ao som": envelope do som (0..1), sobe na hora e cai devagar.
    sound_env: f32,
    /// Defasagem do efeito em relação às outras saídas do grupo (ciclos).
    shift: f32,
    strip: Vec<[f32; 3]>,
    out: Vec<Color>,
    display: Vec<Color>,
    current: f32,
}

/// Constante de tempo da cor da tela nas fans: muda devagar, sem piscar.
const SCREEN_COLOR_TAU: f32 = 0.4;
/// Queda do brilho no "Reagir ao som" depois de uma batida (s).
const SOUND_RELEASE: f32 = 0.3;

impl OutState {
    fn new(cfg: OutputCfg, color: &ColorCfg, aspect: f32, old: Option<&mut OutState>) -> OutState {
        let rects = zones::layout(&cfg.zones, aspect);
        let map = zones::led_map(&cfg.leds, rects.len());
        let pos = visuals::positions(&cfg.leds);
        let adjust = Some(Adjust::for_output(color, &cfg));
        let mut s = OutState { cfg, rects, map, pos, adjust, ..Default::default() };
        // Mudou um ajuste: continua de onde estava em vez de dar um salto.
        if let Some(old) = old {
            if old.smooth.len() == s.rects.len() {
                s.smooth = std::mem::take(&mut old.smooth);
                s.targets = std::mem::take(&mut old.targets);
            }
            s.dom = old.dom;
            s.dom_target = old.dom_target;
            s.music = std::mem::take(&mut old.music);
            s.sound_env = old.sound_env;
        }
        s
    }

    fn render(&mut self, t: f32, dt: f32, smoothing: f32, levels: audio::Levels, spectrum: &[f32]) {
        let n = self.cfg.leds.count as usize;
        self.strip.clear();
        self.strip.resize(n, [0.0; 3]);
        let mode = self.cfg.mode;
        if self.cfg.uses_audio() {
            self.music.update(levels, spectrum, self.cfg.audio.sensitivity, dt);
        }
        match mode {
            OutputMode::Off => {}
            OutputMode::Screen => {
                self.targets.resize(self.rects.len(), [0.0; 3]);
                if self.smooth.len() != self.targets.len() {
                    self.smooth = self.targets.clone();
                }
                // Suavização exponencial, independente do FPS.
                let s = smoothing.clamp(0.0, 0.98);
                let alpha = 1.0 - (s * 0.97).powf(dt * 60.0);
                for (cur, tgt) in self.smooth.iter_mut().zip(&self.targets) {
                    for c in 0..3 {
                        cur[c] += (tgt[c] - cur[c]) * alpha;
                    }
                }
                for (px, z) in self.strip.iter_mut().zip(&self.map) {
                    if let Some(z) = z {
                        *px = self.smooth.get(*z).copied().unwrap_or_default();
                    }
                }
            }
            OutputMode::ScreenColor | OutputMode::ScreenAudio => {
                let alpha = 1.0 - (-dt / SCREEN_COLOR_TAU).exp();
                for c in 0..3 {
                    self.dom[c] += (self.dom_target[c] - self.dom[c]) * alpha;
                }
                let c = if self.cfg.sound.vivid { vivid(self.dom) } else { self.dom };
                self.strip.fill(c);
            }
            OutputMode::Effect => {
                // Cascata entre grupos: a mesma animação, adiantada `shift` ciclos.
                let rate = self.cfg.effect.speed.clamp(1.0, 10.0) * 0.1;
                visuals::render_effect(&self.cfg.effect, t + self.shift / rate, &self.pos, &mut self.strip)
            }
            OutputMode::Audio => visuals::render_audio(&self.cfg.audio, &self.music, t, &self.pos, &mut self.strip),
        }
        // "Reagir ao som" e Tela + Som: as cores continuam as da tela; só o brilho
        // segue o som. O limite de consumo vem depois, em `apply`, e vale no pico.
        let full = mode == OutputMode::ScreenAudio;
        if full || (self.cfg.sound.enabled && matches!(mode, OutputMode::Screen | OutputMode::ScreenColor)) {
            let target = match self.cfg.sound.band {
                SoundBand::Bass => self.music.beat,
                SoundBand::All => self.music.beat.max((self.music.loudness * 1.2).min(1.0)),
            };
            self.sound_env = if target > self.sound_env {
                target
            } else {
                self.sound_env + (target - self.sound_env) * (1.0 - (-dt / SOUND_RELEASE).exp())
            };
            let k = self.cfg.sound.level(self.sound_env, full);
            self.strip.iter_mut().flatten().for_each(|c| *c *= k);
        }
    }

    fn apply(&mut self, adjust: &Adjust) {
        self.current = adjust.apply(&self.strip, &mut self.out, &mut self.display);
    }
}

/// O que é derivado da configuração e só muda quando ela muda.
#[derive(Default)]
struct Derived {
    capture: CaptureCfg,
    color: ColorCfg,
    outs: Vec<OutState>,
    layout: Vec<(u8, u16, u8)>,
    period: Duration,
    aspect: f32,
}

impl Derived {
    fn new(cfg: &Config, aspect: f32, old: &mut Derived) -> Derived {
        let shifts = cfg.group_shifts();
        let outs = cfg
            .outputs
            .iter()
            .enumerate()
            .map(|(i, o)| {
                let mut s = OutState::new(o.clone(), &cfg.color, aspect, old.outs.get_mut(i));
                s.shift = shifts[i];
                s
            })
            .collect();
        Derived {
            capture: cfg.capture.clone(),
            color: cfg.color.clone(),
            outs,
            layout: cfg.layout(),
            period: Duration::from_secs_f32(1.0 / cfg.capture.fps.clamp(10, 144) as f32),
            aspect,
        }
    }

    fn global_limit(&self) -> Option<f32> {
        self.color.power_limit.then_some(self.color.max_current_ma.max(100) as f32)
    }
}

/// Média móvel de quadros por segundo, atualizada a cada ~0,5 s.
struct Rate {
    count: u32,
    since: Instant,
    value: f32,
}

impl Rate {
    fn new() -> Rate {
        Rate { count: 0, since: Instant::now(), value: 0.0 }
    }
    fn tick(&mut self) {
        self.count += 1;
        let secs = self.since.elapsed().as_secs_f32();
        if secs >= 0.5 {
            self.value = self.count as f32 / secs;
            self.count = 0;
            self.since = Instant::now();
        }
    }
}

fn run(shared: Arc<Shared>, gen: u64, mut outputs: Vec<Box<dyn LedOutput>>) {
    let engine = &shared.engine;
    let alive = || engine.generation.load(Ordering::SeqCst) == gen;
    let started = Instant::now();
    let mut cfg_version = u64::MAX;
    let mut d = Derived { aspect: 16.0 / 9.0, ..Default::default() };

    let mut dup: Option<Duplicator> = None;
    let mut img = Image::default();
    let mut content = [0.0, 0.0, 1.0, 1.0];
    let mut fresh = false;
    let mut audio_on = false;
    let mut analyzer = Analyzer::default();
    let mut spectrum: Vec<f32> = Vec::new();

    let (mut all, mut all_display) = (Vec::new(), Vec::new());
    let (mut fps, mut cap_fps) = (Rate::new(), Rate::new());
    let mut last = Instant::now();
    let mut next = Instant::now();
    let mut present: Option<i64> = None;

    while alive() {
        let t0 = Instant::now();
        let dt = t0.duration_since(last).as_secs_f32().min(0.2);
        last = t0;
        let version = shared.config_version.load(Ordering::SeqCst);
        if version != cfg_version {
            let cfg = shared.config.read().unwrap().clone();
            if dup.is_some() && cfg.capture.monitor != d.capture.monitor {
                dup = None;
            }
            // Mudou pino, quantidade ou ordem: o Arduino precisa saber.
            if cfg_version != u64::MAX && cfg.layout() != d.layout {
                engine.resync.store(true, Ordering::SeqCst);
            }
            d = Derived::new(&cfg, d.aspect, &mut d);
            cfg_version = version;
        }
        if engine.resync.swap(false, Ordering::SeqCst) {
            if let Some(serial) = outputs.first_mut() {
                engine.set_firmware(serial.configure(&d.layout));
            }
        }

        let needs_screen = d.outs.iter().any(|o| o.cfg.mode.uses_screen());
        let needs_audio = d.outs.iter().any(|o| o.cfg.uses_audio());
        if needs_audio != audio_on {
            if needs_audio {
                match shared.audio.start() {
                    Ok(()) => audio_on = true,
                    Err(e) => *engine.error.lock().unwrap() = Some(e),
                }
            } else {
                shared.audio.stop();
                audio_on = false;
            }
        }
        let mut latency = None;

        if needs_screen {
            if dup.is_none() {
                match Duplicator::new(d.capture.monitor) {
                    Ok(x) => dup = Some(x),
                    Err(e) => {
                        *engine.error.lock().unwrap() = Some(e);
                        std::thread::sleep(Duration::from_millis(500));
                        continue;
                    }
                }
            }
            let cap = dup.as_mut().unwrap();
            // Sem espera: o timeout do AcquireNextFrame tem a resolução do
            // timer do Windows (~15,6 ms) e derrubaria o FPS com a tela parada.
            // O DXGI acumula as mudanças, então pegar uma vez por quadro basta.
            match cap.grab(0, &mut img) {
                Ok(Some(presented)) => {
                    present = Some(presented);
                    cap_fps.tick();
                    let aspect = img.width as f32 / img.height.max(1) as f32;
                    if (aspect - d.aspect).abs() > 0.01 {
                        let cfg = shared.config.read().unwrap().clone();
                        d = Derived::new(&cfg, aspect, &mut d);
                    }
                    let work = if d.capture.exclude_taskbar { cap.work_area } else { [0.0, 0.0, 1.0, 1.0] };
                    content = zones::content_area(&img, work, d.capture.ignore_black_bars, content);
                    let mut dominant = None;
                    for o in d.outs.iter_mut() {
                        match o.cfg.mode {
                            OutputMode::Screen => zones::sample(&img, content, &o.rects, &mut o.targets),
                            OutputMode::ScreenColor | OutputMode::ScreenAudio => {
                                o.dom_target = *dominant.get_or_insert_with(|| zones::dominant(&img, content));
                            }
                            _ => {}
                        }
                    }
                    fresh = true;
                }
                Ok(None) => {}
                Err(capture::Error::Lost) => {
                    // Troca de resolução, UAC, tela cheia exclusiva: recria logo.
                    dup = None;
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                Err(capture::Error::Other(e)) => {
                    *engine.error.lock().unwrap() = Some(format!("Captura: {e}"));
                    dup = None;
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            }
        } else if dup.is_some() {
            dup = None;
        }

        let levels = if audio_on {
            spectrum = shared.audio.spectrum(visuals::BANDS, &mut analyzer);
            shared.audio.levels()
        } else {
            audio::Levels::default()
        };

        let t = started.elapsed().as_secs_f32();
        let smoothing = d.color.smoothing;
        let mut total = 0.0;
        for o in d.outs.iter_mut() {
            o.render(t, dt, smoothing, levels, &spectrum);
            let adjust = o.adjust.take().unwrap_or_else(|| Adjust::for_output(&d.color, &o.cfg));
            o.apply(&adjust);
            o.adjust = Some(adjust);
            total += o.current;
        }
        // Limite global por cima dos limites de cada saída: corta todas na
        // mesma proporção.
        if let Some(max) = d.global_limit() {
            if total > max {
                let k = max / total;
                total = 0.0;
                for o in d.outs.iter_mut() {
                    let tighter = o.adjust.as_ref().unwrap().limited(o.current * k);
                    o.apply(&tighter);
                    total += o.current;
                }
            }
        }
        all.clear();
        all_display.clear();
        for o in &d.outs {
            all.extend_from_slice(&o.out);
            all_display.extend_from_slice(&o.display);
        }

        let mut failed = None;
        for o in outputs.iter_mut() {
            if let Err(e) = o.send(&all) {
                failed = Some(e);
                break;
            }
        }
        if let Some(e) = failed {
            engine.fail(gen, e);
            shared.audio.stop();
            return;
        }
        fps.tick();
        // Latência: do quadro aparecer no monitor até sair para os LEDs.
        if let Some(p) = present.take() {
            latency = Some(capture::ms_since(p));
        } else if !needs_screen {
            latency = Some(t0.elapsed().as_secs_f32() * 1000.0);
        }

        {
            let mut f = engine.frame.lock().unwrap();
            f.seq = f.seq.wrapping_add(1);
            if let Some(l) = latency {
                f.latency_ms = l;
            }
            if fresh {
                f.image.width = img.width;
                f.image.height = img.height;
                f.image.bgra.clone_from(&img.bgra);
                fresh = false;
            }
            f.content = content;
            f.zones = d
                .outs
                .iter()
                .map(|o| match o.cfg.mode {
                    OutputMode::Screen => o.smooth.iter().map(|c| to_color(*c)).collect(),
                    _ => Vec::new(),
                })
                .collect();
            f.leds.clone_from(&all_display);
            f.bands = d.outs.iter().find(|o| o.cfg.uses_audio()).map(|o| o.music.bands.clone()).unwrap_or_default();
            f.fps = fps.value;
            f.capture_fps = cap_fps.value;
            f.current_ma = total;
            f.currents = d.outs.iter().map(|o| o.current).collect();
        }

        // Relógio fixo: o próximo quadro sai em next + período, sem acumular atraso.
        next += d.period;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > d.period {
            next = now;
        }
    }

    // Parou: apaga todas as saídas e limpa a prévia.
    let black = vec![Color::default(); d.layout.iter().map(|l| l.1 as usize).sum()];
    for o in outputs.iter_mut() {
        let _ = o.send(&black);
    }
    if audio_on && !engine.is_running() {
        shared.audio.stop();
    }
    if !engine.is_running() {
        *engine.frame.lock().unwrap() = Frame::default();
    }
}

/// "Cor sempre viva": o matiz e a saturação da cor da tela, no brilho máximo.
/// Cena cinza ou preta vira branco quente, com transição suave.
fn vivid(c: [f32; 3]) -> [f32; 3] {
    const WARM: [f32; 3] = [1.0, 0.72, 0.45];
    let max = c[0].max(c[1]).max(c[2]);
    if max < 1e-3 {
        return WARM;
    }
    let sat = (max - c[0].min(c[1]).min(c[2])) / max;
    let k = ((sat - 0.08) / 0.17).clamp(0.0, 1.0);
    let k = k * k * (3.0 - 2.0 * k);
    [0, 1, 2].map(|i| WARM[i] + (c[i] / max - WARM[i]) * k)
}

fn to_color(c: [f32; 3]) -> Color {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Color { r: b(c[0]), g: b(c[1]), b: b(c[2]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::RwLock;

    fn shared(outputs: Vec<OutputCfg>) -> Arc<Shared> {
        let mut config = Config::default();
        config.outputs = outputs;
        Arc::new(Shared {
            config: RwLock::new(config),
            config_version: AtomicU64::new(0),
            config_path: PathBuf::new(),
            engine: Engine::default(),
            audio: crate::audio::Monitor::default(),
            mapper: Mutex::new(None),
        })
    }

    fn run_for(outputs: Vec<OutputCfg>, secs: u64) {
        let s = shared(outputs);
        start(s.clone()).unwrap();
        for _ in 0..secs {
            std::thread::sleep(Duration::from_secs(1));
            let f = s.engine.frame.lock().unwrap();
            println!(
                "{:.1} FPS, {:.1} quadros novos/s, latência {:.2} ms, {:.0} mA {:?}, {} LEDs, faixas {:?}",
                f.fps,
                f.capture_fps,
                f.latency_ms,
                f.current_ma,
                f.currents.iter().map(|c| *c as u32).collect::<Vec<_>>(),
                f.leds.len(),
                f.bands.iter().take(8).map(|b| (b * 9.0) as u8).collect::<Vec<_>>()
            );
        }
        let fps = s.engine.frame.lock().unwrap().fps;
        s.engine.stop();
        s.audio.stop();
        assert!(fps > 55.0, "{fps}");
    }

    fn strip(mode: OutputMode) -> OutputCfg {
        OutputCfg { mode, ..OutputCfg::strip(9) }
    }

    fn fan(mode: OutputMode) -> OutputCfg {
        OutputCfg { mode, parallel: 7, ..OutputCfg::fan(6) }
    }

    /// Saída serial simulada: fita em Tela e fan em Cor da tela ao mesmo tempo.
    /// `cargo test --release laco_tela -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn laco_tela() {
        run_for(vec![fan(OutputMode::ScreenColor), strip(OutputMode::Screen)], 6);
    }

    /// Fita em Tela e fan em Som. Toque uma música e rode:
    /// `cargo test --release laco_musica -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn laco_musica() {
        run_for(vec![fan(OutputMode::Audio), strip(OutputMode::Screen)], 6);
    }

    #[test]
    fn cor_viva_tira_a_escuridao_da_cena() {
        // Vermelho escuro de cena de jogo vira vermelho no máximo.
        let c = vivid([0.12, 0.02, 0.02]);
        assert!((c[0] - 1.0).abs() < 1e-5 && c[1] < 0.2 && c[2] < 0.2, "{c:?}");
        // Cinza e preto viram branco quente.
        assert_eq!(vivid([0.1, 0.1, 0.1]), [1.0, 0.72, 0.45]);
        assert_eq!(vivid([0.0; 3]), [1.0, 0.72, 0.45]);
    }

    /// Sem tela nem som (roda em qualquer máquina): os limites de cada saída e o
    /// global seguram a corrente das 7 fans em paralelo.
    #[test]
    fn limites_por_saida_e_global() {
        let mut white = fan(OutputMode::Effect);
        white.effect.kind = crate::config::EffectKind::Static;
        white.effect.colors = [Color { r: 255, g: 255, b: 255 }; 2];
        white.max_current_ma = 1500;
        let mut strip = strip(OutputMode::Effect);
        strip.effect = white.effect.clone();
        strip.power_limit = false;
        let mut cfg = Config::default();
        cfg.color.brightness = 1.0;
        cfg.color.max_current_ma = 2500;
        cfg.outputs = vec![white, strip];
        let mut d = Derived::new(&cfg, 16.0 / 9.0, &mut Derived::default());
        let mut total = 0.0;
        for o in d.outs.iter_mut() {
            o.render(0.0, 1.0 / 60.0, 0.5, audio::Levels::default(), &[]);
            let a = o.adjust.clone().unwrap();
            o.apply(&a);
            total += o.current;
        }
        assert!(d.outs[0].current <= 1500.0 + 1e-3, "fan acima do limite dela");
        // Fita de 60 LEDs brancos sem limite próprio = 3,6 A; o total passa de 2,5 A.
        assert!(total > 2500.0);
        let k = 2500.0 / total;
        let mut after = 0.0;
        for o in d.outs.iter_mut() {
            let a = o.adjust.as_ref().unwrap().limited(o.current * k);
            o.apply(&a);
            after += o.current;
        }
        assert!(after <= 2500.0 + 1.0, "{after}");
    }
}
