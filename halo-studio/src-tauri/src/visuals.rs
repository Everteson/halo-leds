//! Efeitos e visualizações de música calculados por software. Cada função
//! preenche a fita inteira (cores 0..1) a partir da posição de cada LED.

use crate::audio;
use crate::color::from_color;
use crate::config::{AudioCfg, AudioMode, EffectCfg, EffectKind, LedsCfg};
use std::f32::consts::TAU;

/// Número de faixas do espectro calculadas por quadro.
pub const BANDS: usize = 24;

/// Estado da música entre quadros: normalização pelo pico recente, batida e
/// ondas em andamento.
#[derive(Default)]
pub struct AudioState {
    peak_bass: f32,
    peak_treble: f32,
    peak_db: f32,
    pub beat: f32,
    pub tone: f32,
    /// 0..1 por faixa, já suavizado
    pub bands: Vec<f32>,
    pub loudness: f32,
    /// 0..1: energia do terço mais agudo do espectro.
    pub high: f32,
    ripples: Vec<(f32, f32)>,
    last_hit: f32,
}

impl AudioState {
    const SILENCE: f32 = 0.001;

    /// Atualiza com o som do último quadro. `dt` em segundos.
    pub fn update(&mut self, l: audio::Levels, spectrum_db: &[f32], sensitivity: f32, dt: f32) {
        // Batida e timbre, como no efeito Som do protótipo.
        self.peak_bass = l.bass.max(self.peak_bass * 0.997);
        self.peak_treble = l.treble.max(self.peak_treble * 0.997);
        let gain = sensitivity.clamp(1.0, 10.0) / 5.0;
        let bass = if self.peak_bass < Self::SILENCE { 0.0 } else { (l.bass / self.peak_bass * gain).min(1.0) };
        let hit = ((bass - 0.35) / 0.65).clamp(0.0, 1.0);
        let rising = hit > self.beat + 0.25;
        self.beat = hit.max(self.beat * 0.82);
        let treble = if self.peak_treble < Self::SILENCE { 0.0 } else { (l.treble / self.peak_treble).min(1.0) };
        self.tone += (treble - self.tone) * 0.2;

        // Espectro: faixa dinâmica de `range` dB abaixo do pico recente.
        let max_db = spectrum_db.iter().copied().fold(-200.0, f32::max);
        self.peak_db = max_db.max(self.peak_db - 6.0 * dt).max(-10.0);
        let range = 60.0 - sensitivity.clamp(1.0, 10.0) * 4.0;
        self.bands.resize(spectrum_db.len(), 0.0);
        let mut sum = 0.0;
        for (v, &db) in self.bands.iter_mut().zip(spectrum_db) {
            let target = ((db - (self.peak_db - range)) / range).clamp(0.0, 1.0);
            *v = if target > *v { target } else { *v + (target - *v) * 0.18 };
            sum += *v;
        }
        let avg = sum / spectrum_db.len().max(1) as f32;
        let top = &self.bands[self.bands.len() * 2 / 3..];
        let hi = top.iter().sum::<f32>() / top.len().max(1) as f32;
        self.high = if hi > self.high { hi } else { self.high + (hi - self.high) * 0.15 };
        self.loudness = if avg > self.loudness { avg } else { self.loudness + (avg - self.loudness) * 0.1 };

        // Ondas: nascem nas batidas fortes e andam ~0,9 da fita por segundo.
        self.last_hit += dt;
        if rising && self.beat > 0.5 && self.last_hit > 0.12 {
            self.ripples.push((0.0, self.beat));
            self.last_hit = 0.0;
        }
        for r in &mut self.ripples {
            r.0 += dt * 0.9;
            r.1 *= 1.0 - dt * 1.2;
        }
        self.ripples.retain(|r| r.0 < 1.2 && r.1 > 0.02);
    }
}

/// Onde cada LED fica, para os efeitos.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pos {
    /// 0..1: ângulo em volta do centro (0 no topo, sentido horário) com mapa;
    /// ordem na fita sem mapa.
    pub p: f32,
    /// 0..1: distância até o centro (com mapa) ou até o meio da fita.
    pub d: f32,
    /// -0.5..0.5: coordenadas locais da fan (x para a direita, y para baixo).
    /// Sem mapa, `x` acompanha a fita e `y` é 0.
    pub x: f32,
    pub y: f32,
    /// Em qual fan da sequência o pixel está (0 sem mapa de várias fans).
    pub fan: u16,
}

/// Posição de cada LED. Com o mapa do assistente, cada fan usa as próprias
/// coordenadas (o centro é o da célula dela); sem mapa, tudo segue a ordem.
pub fn positions(cfg: &LedsCfg) -> Vec<Pos> {
    let n = cfg.count.max(1) as usize;
    let fans = cfg.fans.max(1) as f32;
    (0..n)
        .map(|i| match cfg.points.get(i).and_then(|pts| pts.first()) {
            Some(&[x, y]) => {
                let (dx, dy, fan) = if cfg.shape == "fan" {
                    // Cada fan ocupa uma célula quadrada; o centro é o da célula.
                    let cell = (x * fans).floor().clamp(0.0, fans - 1.0);
                    (x * fans - cell - 0.5, y - 0.5, cell as u16)
                } else {
                    (x - 0.5, y - 0.5, 0)
                };
                let p = (dy.atan2(dx) / TAU + 1.25).fract();
                Pos { p, d: (dx.hypot(dy) / 0.5).min(1.0), x: dx, y: dy, fan }
            }
            None => {
                let p = (i as f32 + 0.5) / n as f32;
                let fan = if cfg.shape == "fan" { (i * cfg.fans.max(1) as usize / n) as u16 } else { 0 };
                Pos { p, d: (2.0 * p - 1.0).abs(), x: p - 0.5, y: 0.0, fan }
            }
        })
        .collect()
}

pub fn render_audio(cfg: &AudioCfg, s: &AudioState, t: f32, pos: &[Pos], out: &mut [[f32; 3]]) {
    let c0 = from_color(cfg.colors[0]);
    let c1 = from_color(cfg.colors[1]);
    for (i, px) in out.iter_mut().enumerate() {
        // Distância do centro (0 no meio, 1 nas pontas ou na borda da fan).
        let d = pos.get(i).map_or(0.0, |q| q.d);
        *px = match cfg.mode {
            AudioMode::Pulse => scale(mix(c0, c1, s.tone), 0.04 + 0.96 * s.beat),
            AudioMode::Spectrum => {
                let f = d * (s.bands.len().max(1) - 1) as f32;
                let (k, frac) = (f as usize, f.fract());
                let v = lerp(band(s, k), band(s, k + 1), frac);
                scale(mix(c0, c1, d), v * v)
            }
            AudioMode::Energy => {
                let level = (s.loudness * 1.3).min(1.0);
                let lit = ((level - d) * 12.0).clamp(0.0, 1.0);
                let heat = if d < 0.6 { mix(c0, c1, d / 0.6) } else { mix(c1, [0.88, 0.41, 0.41], (d - 0.6) / 0.4) };
                scale(heat, lit * (0.35 + 0.65 * s.beat.max(0.4)))
            }
            AudioMode::Ripple => {
                let glow: f32 = s.ripples.iter().map(|&(r, a)| a * (-((d - r) * 9.0).powi(2)).exp()).sum();
                let base = scale(c0, 0.03 + 0.05 * s.loudness);
                add(base, scale(hsv((t * 0.05 + d * 0.3 + s.tone * 0.5).fract(), 0.85), glow.min(1.0)))
            }
            AudioMode::Space => {
                // Anel interno pulsa com os graves; moldura acompanha os agudos.
                if d < INNER_RING {
                    scale(c0, 0.04 + 0.96 * s.beat)
                } else {
                    scale(c1, 0.04 + 0.96 * (s.high * 1.3).min(1.0))
                }
            }
        };
    }
}

/// Raio que separa o anel interno da moldura externa da fan.
const INNER_RING: f32 = 0.62;

pub fn render_effect(cfg: &EffectCfg, t: f32, pos: &[Pos], out: &mut [[f32; 3]]) {
    let c0 = from_color(cfg.colors[0]);
    let c1 = from_color(cfg.colors[1]);
    let rate = cfg.speed.clamp(1.0, 10.0) * 0.1; // ciclos por segundo
    let size = cfg.size.clamp(0.0, 1.0);
    let lag = cfg.fan_phase.clamp(0.0, 1.0);
    let angle = cfg.angle.to_radians();
    for (i, (px, q)) in out.iter_mut().zip(pos).enumerate() {
        // Defasagem entre fans: 100% = um ciclo inteiro de diferença por fan.
        let ph = t * rate + q.fan as f32 * lag;
        let p = q.p;
        let c = match cfg.kind {
            EffectKind::Static => c0,
            EffectKind::Breathing => scale(c0, 0.5 - 0.5 * (ph * TAU).cos()),
            EffectKind::Rainbow => hsv((ph * 0.5).fract(), 1.0),
            EffectKind::Wave => hsv((ph * 0.5 + p).fract(), 1.0),
            EffectKind::Gradient => mix(c0, c1, 0.5 - 0.5 * ((p + ph * 0.5) * TAU).cos()),
            EffectKind::Comet => {
                let head = (ph * 0.5).fract();
                let tail = (-(head - p).rem_euclid(1.0) * (6.0 + 20.0 * (1.0 - size))).exp();
                add(scale(c1, 0.04), scale(c0, tail))
            }
            EffectKind::Orbit => {
                // Anel interno no sentido horário, moldura no anti-horário.
                let inner = q.d < INNER_RING;
                let head = if inner { (ph * 0.5).fract() } else { (-ph * 0.5).rem_euclid(1.0) };
                let behind = if inner { (head - p).rem_euclid(1.0) } else { (p - head).rem_euclid(1.0) };
                let tail = (-behind * (3.0 + 18.0 * (1.0 - size))).exp();
                scale(if inner { c0 } else { c1 }, 0.05 + 0.95 * tail)
            }
            EffectKind::Ripples => {
                let w = 0.08 + 0.22 * size;
                let v: f32 = (0..3)
                    .map(|k| {
                        let r = (ph * 0.5 + k as f32 / 3.0).fract() * 1.3;
                        (-((q.d - r) / w).powi(2)).exp() * (1.0 - r / 1.3)
                    })
                    .sum();
                add(scale(c1, 0.04), scale(mix(c0, c1, q.d), v.min(1.0)))
            }
            EffectKind::Radar => {
                let head = (ph * 0.5).fract();
                let behind = (head - p).rem_euclid(1.0);
                let tail = (-behind * (2.5 + 14.0 * (1.0 - size))).exp();
                add(scale(c1, 0.03), scale(mix(c1, c0, tail), tail))
            }
            EffectKind::Seesaw => {
                // Lados trocam de cor suavemente; o eixo gira devagar.
                let axis = angle + ph * 0.1 * TAU;
                let proj = q.x * axis.cos() + q.y * axis.sin();
                let side = (proj * (4.0 + 30.0 * (1.0 - size))).tanh();
                mix(c0, c1, 0.5 + 0.5 * side * (ph * 0.5 * TAU).cos())
            }
            EffectKind::Scanner => {
                let proj = q.x * angle.cos() + q.y * angle.sin();
                let at = (ph * 0.5 * TAU).sin() * 0.55;
                let v = (-((proj - at) / (0.05 + 0.25 * size)).powi(2)).exp();
                add(scale(c1, 0.04), scale(c0, v))
            }
            EffectKind::YinYang => {
                let a = ((p - ph * 0.25) * TAU).cos();
                let soft = 0.05 + 0.6 * size;
                mix(c0, c1, smooth(-soft, soft, a))
            }
            EffectKind::Fire => {
                // Mais quente embaixo (y para baixo), com chamas subindo.
                let base = 0.3 + 0.7 * (q.y + 0.5);
                let n1 = noise3(q.x * 3.0, q.y * 3.0 + ph * 2.0, ph * 0.7);
                let n2 = noise3(q.x * 7.0 + 7.0, q.y * 7.0 + ph * 3.5, 3.0 + ph);
                fire(base * (0.35 + 0.65 * n1) + 0.25 * (n2 - 0.5) + 0.25 * (size - 0.5))
            }
            EffectKind::Aurora => {
                let n = noise3(q.x * 2.0 + ph * 0.3, q.y * 2.0, ph * 0.4);
                let m = noise3(q.x * 3.0 + 5.0, q.y * 3.0 - ph * 0.25, 10.0 + ph * 0.3);
                scale(mix(c0, c1, smooth(0.2, 0.8, n)), 0.15 + 0.85 * m.powf(0.6 + size * 1.4))
            }
            EffectKind::Twinkle => {
                // Cada pixel tem seu ritmo, tirado de um hash fixo (determinístico).
                let (h1, h2) = (hash(i as i32, 1, 0), hash(i as i32, 2, 0));
                let s = ((ph * (0.5 + h1) + h2) * TAU).sin().max(0.0).powf(2.0 + 14.0 * (1.0 - size));
                add(scale(c1, 0.06), scale(mix(c0, [1.0; 3], 0.35), s))
            }
            EffectKind::Heartbeat => {
                let pulse = |x: f32| (-(x / (0.035 + 0.05 * size)).powi(2)).exp();
                // A batida sai do centro: quanto mais longe, mais atrasada.
                let cyc = (ph * 0.6 - q.d * 0.12).rem_euclid(1.0);
                let env = pulse(cyc - 0.1) + 0.6 * pulse(cyc - 0.3);
                add(scale(c1, 0.05), scale(c0, env.min(1.0)))
            }
            EffectKind::Spiral => hsv((p + q.d * (0.4 + 1.6 * size) - ph * 0.5).rem_euclid(1.0), 1.0),
            EffectKind::Kaleido => {
                let n = 3.0 + (size * 5.0).round();
                let a = ((p - ph * 0.1) * n).rem_euclid(1.0);
                let fold = a.min(1.0 - a) * 2.0;
                let hue = (fold * 0.35 + q.d * 0.4 + ph * 0.15).rem_euclid(1.0);
                scale(mix(hsv(hue, 0.9), c0, 0.25), 1.0 - 0.6 * fold)
            }
        };
        *px = c.map(|v| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 });
    }
}

/// Paleta de fogo: preto → vermelho → laranja → amarelo.
fn fire(h: f32) -> [f32; 3] {
    let h = h.clamp(0.0, 1.0);
    [(h * 2.2).min(1.0), ((h * 2.2 - 0.9).clamp(0.0, 1.0)) * 0.85, (h * 3.0 - 2.4).clamp(0.0, 1.0) * 0.5]
}

fn smooth(e0: f32, e1: f32, x: f32) -> f32 {
    let k = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    k * k * (3.0 - 2.0 * k)
}

/// Hash inteiro → 0..1, com semente fixa para os efeitos serem repetíveis.
fn hash(x: i32, y: i32, z: i32) -> f32 {
    const SEED: u32 = 0x4A10_5EED;
    let mut h = (x as u32).wrapping_mul(0x8DA6_B343) ^ (y as u32).wrapping_mul(0xD816_3841) ^ (z as u32).wrapping_mul(0xCB1A_B31F) ^ SEED;
    h ^= h >> 13;
    h = h.wrapping_mul(0x5BD1_E995);
    h ^= h >> 15;
    (h & 0xFF_FFFF) as f32 / 16_777_215.0
}

/// Ruído de valor 3D suave (0..1): barato e orgânico o bastante para LEDs.
fn noise3(x: f32, y: f32, z: f32) -> f32 {
    let (xi, yi, zi) = (x.floor(), y.floor(), z.floor());
    let s = |v: f32| v * v * (3.0 - 2.0 * v);
    let (fx, fy, fz) = (s(x - xi), s(y - yi), s(z - zi));
    let (xi, yi, zi) = (xi as i32, yi as i32, zi as i32);
    let corner = |dx, dy, dz| hash(xi + dx, yi + dy, zi + dz);
    let plane = |dz| {
        let a = lerp(corner(0, 0, dz), corner(1, 0, dz), fx);
        let b = lerp(corner(0, 1, dz), corner(1, 1, dz), fx);
        lerp(a, b, fy)
    };
    lerp(plane(0), plane(1), fz)
}

fn band(s: &AudioState, k: usize) -> f32 {
    s.bands.get(k.min(s.bands.len().saturating_sub(1))).copied().unwrap_or(0.0)
}

fn lerp(a: f32, b: f32, k: f32) -> f32 {
    a + (b - a) * k
}

fn scale(c: [f32; 3], k: f32) -> [f32; 3] {
    [c[0] * k, c[1] * k, c[2] * k]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [(a[0] + b[0]).min(1.0), (a[1] + b[1]).min(1.0), (a[2] + b[2]).min(1.0)]
}

fn mix(a: [f32; 3], b: [f32; 3], k: f32) -> [f32; 3] {
    let k = k.clamp(0.0, 1.0);
    [lerp(a[0], b[0], k), lerp(a[1], b[1], k), lerp(a[2], b[2], k)]
}

/// Matiz `h` (0..1) com saturação `s` e brilho máximo.
fn hsv(h: f32, s: f32) -> [f32; 3] {
    let f = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        1.0 - s * k.min(4.0 - k).clamp(0.0, 1.0)
    };
    [f(5.0), f(3.0), f(1.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posicao_pelo_mapa_ou_pela_ordem() {
        let mut cfg = LedsCfg { count: 4, ..Default::default() };
        let by_index = positions(&cfg);
        assert!((by_index[1].p - 0.375).abs() < 1e-6);
        // Uma fan: topo, direita, baixo e um pixel sem ponto.
        cfg.points = vec![vec![[0.5, 0.1]], vec![[0.9, 0.5]], vec![[0.5, 0.9]], vec![]];
        let p = positions(&cfg);
        assert!(p[0].p.abs() < 1e-4 || (p[0].p - 1.0).abs() < 1e-4, "{p:?}");
        assert!((p[1].p - 0.25).abs() < 1e-4, "{p:?}");
        assert!((p[2].p - 0.5).abs() < 1e-4, "{p:?}");
        assert!((p[0].d - 0.8).abs() < 1e-4);
        // Duas fans: o ponto da segunda fan gira em volta do centro dela.
        cfg.fans = 2;
        cfg.points[1] = vec![[0.95, 0.5]];
        let q = positions(&cfg)[1];
        assert!((q.p - 0.25).abs() < 1e-4);
        assert_eq!(q.fan, 1);
    }

    /// Todo efeito, em vários instantes, com e sem mapa: cores finitas em 0..1,
    /// e o brilho global continua limitando a saída.
    #[test]
    fn efeitos_ficam_no_intervalo_e_respeitam_o_brilho() {
        use crate::color::Adjust;
        use crate::config::ColorCfg;
        let kinds = [
            EffectKind::Static, EffectKind::Breathing, EffectKind::Rainbow, EffectKind::Wave,
            EffectKind::Gradient, EffectKind::Comet, EffectKind::Orbit, EffectKind::Ripples,
            EffectKind::Radar, EffectKind::Seesaw, EffectKind::Scanner, EffectKind::YinYang,
            EffectKind::Fire, EffectKind::Aurora, EffectKind::Twinkle, EffectKind::Heartbeat,
            EffectKind::Spiral, EffectKind::Kaleido,
        ];
        // Duas fans mapeadas (22 pixels) e uma fita sem mapa.
        let mut fans = LedsCfg { count: 22, fans: 2, ..Default::default() };
        fans.points = (0..22)
            .map(|i| {
                let a = i as f32 / 11.0 * TAU;
                let r = if i % 11 < 6 { 0.2 } else { 0.42 };
                vec![[((i / 11) as f32 + 0.5 + r * a.cos()) / 2.0, 0.5 + r * a.sin()]]
            })
            .collect();
        let strip = LedsCfg { count: 60, ..Default::default() };
        let half = Adjust::new(&ColorCfg { brightness: 0.5, gamma: 1.0, power_limit: false, ..Default::default() });
        for leds in [&fans, &strip] {
            let pos = positions(leds);
            for kind in kinds {
                for size in [0.0, 0.5, 1.0] {
                    let cfg = EffectCfg { kind, size, fan_phase: 0.5, angle: 30.0, ..Default::default() };
                    let mut out = vec![[0.0; 3]; pos.len()];
                    let mut seen = 0.0f32;
                    for k in 0..240 {
                        render_effect(&cfg, k as f32 * 0.137, &pos, &mut out);
                        for c in out.iter().flatten() {
                            assert!(c.is_finite() && (0.0..=1.0).contains(c), "{kind:?}: {c}");
                            seen = seen.max(*c);
                        }
                        let (mut o, mut disp) = (Vec::new(), Vec::new());
                        half.apply(&out, &mut o, &mut disp);
                        assert!(o.iter().all(|c| c.r.max(c.g).max(c.b) <= 128), "{kind:?} passou do brilho");
                    }
                    assert!(seen > 0.2, "{kind:?} (size {size}) não acendeu nada");
                }
            }
        }
    }

    #[test]
    fn orbita_separa_anel_e_moldura() {
        let mut leds = LedsCfg { count: 2, ..Default::default() };
        leds.points = vec![vec![[0.5, 0.35]], vec![[0.5, 0.08]]];
        let pos = positions(&leds);
        let cfg = EffectCfg { kind: EffectKind::Orbit, ..Default::default() };
        let mut out = [[0.0; 3]; 2];
        render_effect(&cfg, 0.0, &pos, &mut out);
        // Anel usa a cor 1 (dourado), moldura a cor 2 (azul).
        assert!(out[0][0] > out[0][2]);
        assert!(out[1][2] > out[1][0]);
    }

    #[test]
    fn arco_iris_comeca_no_vermelho() {
        let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        assert!(near(hsv(0.0, 1.0), [1.0, 0.0, 0.0]));
        assert!(near(hsv(1.0 / 3.0, 1.0), [0.0, 1.0, 0.0]));
    }
}
