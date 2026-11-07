//! Configuração do app, salva em JSON na pasta de configuração do usuário
//! (%APPDATA%\com.evert.halostudio\config.json).
//!
//! Os LEDs ficam em **saídas** (`outputs`): cada uma é um pino do Arduino com
//! uma fan ou uma fita, e tem modo, brilho, limite de consumo e mapeamento
//! próprios. Os LEDs de todas as saídas vão em sequência no quadro Adalight,
//! na ordem da lista (ver firmware/halo-adalight/PROTOCOLO.md).

use crate::openrgb::Color;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Versão do formato do arquivo. Até a 1 havia uma fita só (`leds`, `zones`,
/// `effect`, `audio` na raiz); a 2 trouxe as saídas.
const FORMAT: u32 = 2;

/// Soma máxima de LEDs que o firmware v2 aceita (o `Hlo?` informa: `max=320`).
pub const MAX_LEDS: u16 = 320;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub capture: CaptureCfg,
    /// Ajustes de cor globais (gama, saturação, temperatura, suavização da
    /// tela). O brilho e o limite daqui valem para o conjunto das saídas.
    pub color: ColorCfg,
    pub outputs: Vec<OutputCfg>,
    pub serial: SerialCfg,
    pub openrgb: OpenRgbCfg,
    pub app: AppCfg,
    /// Ajustes de "Todos os coolers" / "Todas as fitas".
    pub groups: GroupsCfg,
    /// Estado guardado pelo backend (a interface não muda; `set_config` preserva).
    pub state: RunState,
    // Formato antigo: só lido para migrar, nunca gravado.
    #[serde(skip_serializing)]
    zones: Option<ZonesCfg>,
    #[serde(skip_serializing)]
    leds: Option<LedsCfg>,
    #[serde(skip_serializing)]
    effect: Option<EffectCfg>,
    #[serde(skip_serializing)]
    audio: Option<AudioCfg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppCfg {
    /// Ao abrir com o Windows, religar as luzes se estavam ligadas.
    pub resume: bool,
}

impl Default for AppCfg {
    fn default() -> Self {
        AppCfg { resume: true }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RunState {
    /// As luzes estavam ligadas na última vez (Iniciar/Parar pelo usuário).
    pub was_running: bool,
    /// O início automático já foi ligado na primeira execução instalada.
    pub autostart_done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureCfg {
    /// Índice na lista de `capture::list_monitors`.
    pub monitor: u32,
    /// Quadros por segundo enviados aos LEDs.
    pub fps: u32,
    pub exclude_taskbar: bool,
    /// Ignora as faixas pretas de filmes (letterbox) ao calcular as bordas.
    pub ignore_black_bars: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputKind {
    Fan,
    Strip,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    Off,
    /// Fita: cada LED mostra a zona da borda da tela que fica atrás dele.
    Screen,
    /// Fan: a cor dominante da tela inteira, com suavização lenta.
    ScreenColor,
    Effect,
    Audio,
    /// Fan: a cor da tela com a intensidade pelo som.
    ScreenAudio,
}

impl OutputMode {
    pub fn uses_screen(self) -> bool {
        matches!(self, OutputMode::Screen | OutputMode::ScreenColor | OutputMode::ScreenAudio)
    }
    pub fn uses_audio(self) -> bool {
        matches!(self, OutputMode::Audio | OutputMode::ScreenAudio)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputCfg {
    pub name: String,
    /// Pino digital do Arduino (2..=12).
    pub pin: u8,
    pub kind: OutputKind,
    /// 0=RGB 1=RBG 2=GRB 3=GBR 4=BRG 5=BGR (WS2812B = GRB).
    pub color_order: u8,
    /// Fans ligadas em paralelo no mesmo fio de dados: recebem as mesmas cores,
    /// mas cada uma puxa corrente. Na fita é 1.
    pub parallel: u16,
    /// LEDs físicos que acendem por pixel (na fan com espelho infinito, até 2).
    pub leds_per_pixel: u8,
    pub mode: OutputMode,
    /// 0..1, multiplica o brilho global.
    pub brightness: f32,
    pub power_limit: bool,
    pub max_current_ma: u32,
    /// Quantidade de pixels (`leds.count`) e mapeamento: zonas na fita, pontos
    /// do assistente na fan.
    pub leds: LedsCfg,
    /// Zonas da tela que esta fita mostra.
    pub zones: ZonesCfg,
    pub effect: EffectCfg,
    pub audio: AudioCfg,
    /// Opcional nos modos de tela (Tela da fita, Cor da tela da fan): o brilho
    /// acompanha o som, mantendo as cores da tela.
    pub sound: SoundCfg,
    /// Segue "Todos" (os ajustes de modo, efeito, som e brilho são os do grupo).
    /// Falso = esta saída tem ajustes próprios.
    pub follow: bool,
    /// Posição na cascata entre as saídas do mesmo tipo (frente → topo → traseira).
    pub cascade: u16,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupsCfg {
    pub fan: GroupCfg,
    pub strip: GroupCfg,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct GroupCfg {
    /// 0..1: defasagem do efeito entre as saídas que seguem "Todos", na ordem
    /// da cascata (0 = sincronizadas; 1 = um ciclo inteiro entre uma e a próxima).
    pub phase: f32,
    /// Mexer em "Todos" também muda as saídas com ajustes próprios.
    pub overwrite: bool,
}

impl GroupsCfg {
    pub fn of(&self, kind: OutputKind) -> &GroupCfg {
        match kind {
            OutputKind::Fan => &self.fan,
            OutputKind::Strip => &self.strip,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SoundBand {
    /// Só os graves (batida).
    Bass,
    /// Graves e o volume geral.
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundCfg {
    pub enabled: bool,
    /// 0..1: quanto o som puxa o brilho (0 = nada).
    pub amount: f32,
    /// 0..1: brilho no silêncio, para a fita não apagar.
    pub floor: f32,
    pub band: SoundBand,
    /// 0..3: "Força do grave", ganho sobre o nível do som (1 = como chega).
    pub boost: f32,
    /// Fans na cor da tela: usa só matiz e saturação, no brilho máximo, para o
    /// brilho vir do som e não da escuridão da cena.
    pub vivid: bool,
}

impl Default for SoundCfg {
    fn default() -> Self {
        SoundCfg { enabled: false, amount: 0.6, floor: 0.4, band: SoundBand::Bass, boost: 1.5, vivid: true }
    }
}

impl SoundCfg {
    /// Multiplicador de brilho para um nível de som `env` (0..1). `full` =
    /// modo Tela + Som, que vai sempre do mínimo ao máximo (ignora a intensidade).
    pub fn level(&self, env: f32, full: bool) -> f32 {
        let floor = self.floor.clamp(0.0, 1.0);
        let amount = if full { 1.0 } else { self.amount.clamp(0.0, 1.0) };
        let env = (env * self.boost.clamp(0.0, 3.0)).clamp(0.0, 1.0);
        1.0 - amount * (1.0 - floor) * (1.0 - env)
    }
}

/// Quantas zonas há em cada borda da tela. A ordem das zonas é sempre no
/// sentido horário a partir do canto inferior esquerdo: esquerda (de baixo
/// para cima), topo (da esquerda para a direita), direita (de cima para
/// baixo) e base (da direita para a esquerda).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ZonesCfg {
    pub left: u16,
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    /// Profundidade de cada zona para dentro da tela, em fração da altura.
    pub depth: f32,
    /// Só para a interface lembrar a distribuição escolhida.
    pub preset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LedsCfg {
    /// Pixels desta saída.
    pub count: u16,
    /// Gira o mapeamento: o LED 0 recebe a zona `offset`.
    pub offset: i32,
    /// Fita montada no sentido anti-horário.
    pub reverse: bool,
    /// Mapeamento manual LED→zona (-1 = apagado). Vazio = distribuir sozinho.
    pub map: Vec<i32>,
    /// Posição de cada pixel no desenho do assistente de mapeamento (0..1).
    /// Cada pixel tem 0 pontos (não acendeu), 1 ou 2 (um LED físico extra que
    /// acende junto). Vazio = os efeitos seguem a ordem dos índices.
    pub points: Vec<Vec<[f32; 2]>>,
    /// Desenho usado no mapeamento: "fan" ou "strip" (segue o tipo da saída).
    pub shape: String,
    /// Quantas fans em sequência no desenho (só para shape = "fan").
    pub fans: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorCfg {
    /// 0..1
    pub brightness: f32,
    pub gamma: f32,
    /// 0..2 (1 = sem mudança)
    pub saturation: f32,
    /// Kelvin; 6500 = neutro
    pub temperature: u32,
    /// 0..1: 0 = sem suavização, 1 = bem lento
    pub smoothing: f32,
    /// Limite do total de todas as saídas juntas.
    pub power_limit: bool,
    pub max_current_ma: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioMode {
    /// Toda a fita pulsa com os graves.
    Pulse,
    /// Barras de frequência espelhadas.
    Spectrum,
    /// Medidor de volume que cresce a partir do centro.
    Energy,
    /// Cada batida solta uma onda que corre pela fita.
    Ripple,
    /// "Volume no espaço": o anel interno pulsa com os graves e a moldura
    /// brilha com os agudos (usa o raio do mapa da fan).
    Space,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioCfg {
    pub mode: AudioMode,
    /// 1..10
    pub sensitivity: f32,
    pub colors: [Color; 2],
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectKind {
    Static,
    Breathing,
    Rainbow,
    Wave,
    Gradient,
    Comet,
    /// Anel interno gira num sentido e a moldura no outro.
    Orbit,
    /// Anéis de luz saindo do centro, como pedra na água.
    Ripples,
    /// Feixe girando com rastro.
    Radar,
    /// Dois lados trocando de cor; o eixo gira devagar.
    Seesaw,
    /// Faixa de luz indo e voltando num ângulo.
    Scanner,
    /// Metade de cada cor, girando, com borda suave.
    YinYang,
    Fire,
    Aurora,
    /// Brilhos que acendem e somem.
    Twinkle,
    /// Pulso duplo saindo do centro.
    Heartbeat,
    Spiral,
    Kaleido,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EffectCfg {
    pub kind: EffectKind,
    pub colors: [Color; 2],
    /// 1..10
    pub speed: f32,
    /// 0..1: tamanho do rastro, da faixa ou da borda, conforme o efeito.
    pub size: f32,
    /// Graus: direção da Varredura e eixo inicial da Gangorra.
    pub angle: f32,
    /// 0..1: defasagem entre fans em sequência (0 = sincronizadas).
    pub fan_phase: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SerialCfg {
    pub port: String,
    pub baud: u32,
    /// Não abre a porta: só monta os quadros Adalight e registra no log.
    pub simulate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OpenRgbCfg {
    pub enabled: bool,
    pub host: String,
    pub port: u16,
    /// Índices dos dispositivos escolhidos (só os com Direct sem auto-save são usados).
    pub devices: Vec<u32>,
}

const GOLD: Color = Color { r: 0xC9, g: 0xA5, b: 0x6B };
const BLUE: Color = Color { r: 0x2E, g: 0x7C, b: 0xD6 };

/// Limite padrão de cada saída nova (6 A, validado com fonte via Molex).
const DEFAULT_OUTPUT_MA: u32 = 6000;

impl Default for CaptureCfg {
    fn default() -> Self {
        CaptureCfg { monitor: 0, fps: 60, exclude_taskbar: false, ignore_black_bars: true }
    }
}

impl Default for ZonesCfg {
    fn default() -> Self {
        // 60 zonas numa tela 16:9.
        ZonesCfg { left: 11, top: 19, right: 11, bottom: 19, depth: 0.12, preset: "full".into() }
    }
}

impl Default for LedsCfg {
    fn default() -> Self {
        LedsCfg {
            count: 60,
            offset: 0,
            reverse: false,
            map: Vec::new(),
            points: Vec::new(),
            shape: "fan".into(),
            fans: 1,
        }
    }
}

impl Default for ColorCfg {
    fn default() -> Self {
        ColorCfg {
            brightness: 0.8,
            gamma: 2.2,
            saturation: 1.0,
            temperature: 6500,
            smoothing: 0.5,
            power_limit: true,
            max_current_ma: 12000,
        }
    }
}

impl Default for AudioCfg {
    fn default() -> Self {
        AudioCfg { mode: AudioMode::Spectrum, sensitivity: 5.0, colors: [BLUE, GOLD] }
    }
}

impl Default for EffectCfg {
    fn default() -> Self {
        EffectCfg { kind: EffectKind::Rainbow, colors: [GOLD, BLUE], speed: 4.0, size: 0.5, angle: 0.0, fan_phase: 0.0 }
    }
}

impl Default for SerialCfg {
    fn default() -> Self {
        SerialCfg { port: String::new(), baud: 1_000_000, simulate: true }
    }
}

impl Default for OpenRgbCfg {
    fn default() -> Self {
        OpenRgbCfg { enabled: false, host: "127.0.0.1".into(), port: 6742, devices: Vec::new() }
    }
}

impl Default for OutputCfg {
    fn default() -> Self {
        OutputCfg::fan(6)
    }
}

impl OutputCfg {
    /// Uma fan Acegeek: 11 pixels, até 2 LEDs físicos por pixel.
    pub fn fan(pin: u8) -> OutputCfg {
        OutputCfg {
            name: "Fans".into(),
            pin,
            kind: OutputKind::Fan,
            color_order: 2,
            parallel: 1,
            leds_per_pixel: 2,
            mode: OutputMode::Effect,
            brightness: 1.0,
            power_limit: true,
            max_current_ma: DEFAULT_OUTPUT_MA,
            leds: LedsCfg { count: 11, shape: "fan".into(), ..Default::default() },
            zones: ZonesCfg::default(),
            effect: EffectCfg::default(),
            audio: AudioCfg { mode: AudioMode::Space, ..Default::default() },
            sound: SoundCfg::default(),
            follow: true,
            cascade: 0,
        }
    }

    pub fn strip(pin: u8) -> OutputCfg {
        OutputCfg {
            name: "Fita".into(),
            kind: OutputKind::Strip,
            leds_per_pixel: 1,
            mode: OutputMode::Screen,
            leds: LedsCfg { count: 60, shape: "strip".into(), ..Default::default() },
            audio: AudioCfg::default(),
            ..OutputCfg::fan(pin)
        }
    }

    /// O som entra nesta saída: modos de música, ou tela com "Reagir ao som".
    pub fn uses_audio(&self) -> bool {
        self.mode.uses_audio()
            || (self.sound.enabled && matches!(self.mode, OutputMode::Screen | OutputMode::ScreenColor))
    }

    /// Multiplicador da corrente estimada por pixel: fans em paralelo × LEDs
    /// físicos por pixel.
    pub fn current_scale(&self) -> f32 {
        self.parallel.max(1) as f32 * self.leds_per_pixel.max(1) as f32
    }

    /// Corrente com todos os pixels em branco total, sem limite (mA).
    pub fn full_white_ma(&self) -> f32 {
        self.leds.count as f32 * 3.0 * crate::color::MA_PER_CHANNEL * self.current_scale()
    }
}

impl ZonesCfg {
    pub fn total(&self) -> usize {
        (self.left + self.top + self.right + self.bottom) as usize
    }
}

impl Config {
    pub fn load(path: &Path) -> Config {
        let mut cfg: Config = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        cfg.migrate();
        cfg.normalize();
        cfg
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("Não deu para criar {}: {e}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("Não deu para salvar {}: {e}", path.display()))
    }

    /// Formato antigo (uma fita só) → uma saída Fan no D6 com 11 pixels, que é
    /// o que estava ligado. Instalação nova também cai aqui.
    pub fn migrate(&mut self) {
        if self.version >= FORMAT {
            return;
        }
        let mut fan = OutputCfg::fan(6);
        if let Some(leds) = self.leds.take() {
            fan.leds.points = leds.points.into_iter().take(11).collect();
            fan.leds.fans = 1;
            // Hoje são 7 fans em paralelo no mesmo fio de dados.
            fan.parallel = 7;
        }
        if let Some(effect) = self.effect.take() {
            fan.effect = effect;
        }
        if let Some(audio) = self.audio.take() {
            fan.audio = AudioCfg { mode: AudioMode::Space, ..audio };
        }
        if let Some(zones) = self.zones.take() {
            fan.zones = zones;
        }
        self.outputs = vec![fan];
        self.version = FORMAT;
    }

    /// Mantém os campos coerentes (forma do desenho, pino, quantidade).
    pub fn normalize(&mut self) {
        for o in &mut self.outputs {
            o.pin = o.pin.clamp(2, 12);
            o.color_order = o.color_order.min(5);
            o.leds.count = o.leds.count.clamp(1, MAX_LEDS);
            o.leds.shape = match o.kind {
                OutputKind::Fan => "fan".into(),
                OutputKind::Strip => "strip".into(),
            };
            if o.kind == OutputKind::Strip {
                o.parallel = 1;
            }
        }
        self.version = FORMAT;
    }

    /// (pino, quantidade, ordem de cor) de cada saída, na ordem do quadro.
    pub fn layout(&self) -> Vec<(u8, u16, u8)> {
        self.outputs.iter().map(|o| (o.pin, o.leds.count, o.color_order)).collect()
    }

    /// Defasagem (em ciclos do efeito) de cada saída: as que seguem "Todos" e
    /// estão em Efeito ficam em cascata, na ordem `cascade` (empate: ordem da lista).
    pub fn group_shifts(&self) -> Vec<f32> {
        let mut shifts = vec![0.0; self.outputs.len()];
        for kind in [OutputKind::Fan, OutputKind::Strip] {
            let phase = self.groups.of(kind).phase.clamp(0.0, 1.0);
            let mut members: Vec<usize> = (0..self.outputs.len())
                .filter(|&i| {
                    let o = &self.outputs[i];
                    o.kind == kind && o.follow && o.mode == OutputMode::Effect
                })
                .collect();
            members.sort_by_key(|&i| (self.outputs[i].cascade, i));
            for (rank, i) in members.into_iter().enumerate() {
                shifts[i] = rank as f32 * phase;
            }
        }
        shifts
    }

    /// Onde cada saída começa no quadro Adalight.
    pub fn offsets(&self) -> Vec<usize> {
        let mut at = 0;
        self.outputs
            .iter()
            .map(|o| {
                let start = at;
                at += o.leds.count as usize;
                start
            })
            .collect()
    }

    pub fn total_leds(&self) -> usize {
        self.outputs.iter().map(|o| o.leds.count as usize).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migra_a_fita_antiga_para_uma_fan_no_d6() {
        let old = r#"{
            "leds": {"count": 17, "points": [[[0.5,0.1]],[[0.9,0.5]],[],[],[],[],[],[],[],[],[],[[0.1,0.1]],[],[],[],[],[]], "shape": "fan", "fans": 1},
            "effect": {"kind": "orbit", "speed": 7},
            "color": {"brightness": 1, "power_limit": false, "max_current_ma": 8000},
            "serial": {"port": "COM4", "simulate": false}
        }"#;
        let mut cfg: Config = serde_json::from_str(old).unwrap();
        cfg.migrate();
        cfg.normalize();
        assert_eq!(cfg.outputs.len(), 1);
        let fan = &cfg.outputs[0];
        assert_eq!((fan.pin, fan.kind, fan.leds.count, fan.color_order), (6, OutputKind::Fan, 11, 2));
        assert_eq!(fan.leds.points.len(), 11);
        assert_eq!(fan.effect.kind, EffectKind::Orbit);
        assert!(fan.power_limit, "saída nova sempre com limite");
        assert_eq!(fan.parallel, 7);
        assert_eq!(cfg.serial.port, "COM4");
        // O formato antigo não volta a ser gravado.
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(!json.contains("\"leds\":{\"count\":17"));
        assert!(json.contains("\"version\":2"));
    }

    #[test]
    fn layout_e_offsets_seguem_a_ordem_das_saidas() {
        let mut cfg = Config::default();
        cfg.migrate();
        cfg.outputs.push(OutputCfg::strip(9));
        cfg.outputs[1].leds.count = 120;
        assert_eq!(cfg.layout(), vec![(6, 11, 2), (9, 120, 2)]);
        assert_eq!(cfg.offsets(), vec![0, 11]);
        assert_eq!(cfg.total_leds(), 131);
    }

    #[test]
    fn reagir_ao_som_respeita_intensidade_e_minimo() {
        let s = SoundCfg { enabled: true, amount: 1.0, floor: 0.4, band: SoundBand::Bass, boost: 1.0, vivid: true };
        assert!((s.level(0.0, false) - 0.4).abs() < 1e-6, "silêncio fica no mínimo");
        assert!((s.level(1.0, false) - 1.0).abs() < 1e-6, "pico em brilho total");
        let off = SoundCfg { amount: 0.0, ..s.clone() };
        assert_eq!(off.level(0.0, false), 1.0, "intensidade 0 não mexe");
        assert!((off.level(0.0, true) - 0.4).abs() < 1e-6, "Tela + Som ignora a intensidade");
        let full = SoundCfg { amount: 1.0, floor: 0.0, ..s.clone() };
        assert!((full.level(0.25, false) - 0.25).abs() < 1e-6);
        // Força do grave: 300% leva um grave fraco (1/3) ao brilho total.
        let strong = SoundCfg { floor: 0.0, boost: 3.0, ..s };
        assert!((strong.level(1.0 / 3.0, true) - 1.0).abs() < 1e-5);
        assert!((strong.level(0.1, true) - 0.3).abs() < 1e-5);
        // Config antiga sem o campo: desligado.
        let o: OutputCfg = serde_json::from_str(r#"{"kind":"strip","mode":"screen"}"#).unwrap();
        assert!(!o.sound.enabled && !o.uses_audio());
    }

    #[test]
    fn cascata_entre_coolers_segue_a_ordem() {
        let mut cfg = Config::default();
        cfg.migrate();
        cfg.outputs = vec![OutputCfg::fan(6), OutputCfg::fan(7), OutputCfg::fan(8), OutputCfg::strip(9)];
        // Ordem: traseira (D6) por último, topo (D8) no meio, frente (D7) primeiro.
        cfg.outputs[0].cascade = 2;
        cfg.outputs[1].cascade = 0;
        cfg.outputs[2].cascade = 1;
        cfg.groups.fan.phase = 0.25;
        assert_eq!(cfg.group_shifts(), vec![0.5, 0.0, 0.25, 0.0]);
        // Com ajustes próprios, sai da cascata.
        cfg.outputs[1].follow = false;
        assert_eq!(cfg.group_shifts(), vec![0.25, 0.0, 0.0, 0.0]);
        // Sincronizado: todos iguais.
        cfg.groups.fan.phase = 0.0;
        assert!(cfg.group_shifts().iter().all(|s| *s == 0.0));
        // Config antiga sem os campos: segue Todos.
        let o: OutputCfg = serde_json::from_str(r#"{"kind":"fan"}"#).unwrap();
        assert!(o.follow);
    }

    #[test]
    fn corrente_considera_fans_em_paralelo() {
        let mut fan = OutputCfg::fan(6);
        fan.parallel = 7;
        // 11 pixels × 3 canais × 20 mA × 7 fans × 2 LEDs = 9,24 A
        assert!((fan.full_white_ma() - 9240.0).abs() < 1.0);
    }
}
