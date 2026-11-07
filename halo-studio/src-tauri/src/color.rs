//! Ajustes finais de cor aplicados a todo quadro antes de ir para os LEDs:
//! saturação, temperatura de cor, brilho, gama e limite de corrente.

use crate::config::{ColorCfg, OutputCfg};
use crate::openrgb::Color;

/// Corrente aproximada de um canal de um WS2812B aceso no máximo.
pub const MA_PER_CHANNEL: f32 = 20.0;

#[derive(Clone)]
pub struct Adjust {
    brightness: f32,
    gamma: f32,
    saturation: f32,
    white: [f32; 3],
    max_ma: Option<f32>,
    /// LEDs físicos por pixel: fans em paralelo × LEDs por pixel.
    scale: f32,
}

impl Adjust {
    pub fn new(cfg: &ColorCfg) -> Adjust {
        Adjust {
            brightness: cfg.brightness.clamp(0.0, 1.0),
            gamma: cfg.gamma.clamp(1.0, 3.5),
            saturation: cfg.saturation.clamp(0.0, 2.0),
            white: white_balance(cfg.temperature),
            max_ma: cfg.power_limit.then_some(cfg.max_current_ma.max(100) as f32),
            scale: 1.0,
        }
    }

    /// Ajustes de uma saída: brilho global × brilho da saída, o limite dela e
    /// a corrente multiplicada pelos LEDs físicos que cada pixel acende.
    pub fn for_output(cfg: &ColorCfg, out: &OutputCfg) -> Adjust {
        Adjust {
            brightness: (cfg.brightness * out.brightness).clamp(0.0, 1.0),
            max_ma: out.power_limit.then_some(out.max_current_ma.max(100) as f32),
            scale: out.current_scale(),
            ..Adjust::new(cfg)
        }
    }

    /// O mesmo ajuste com um teto de corrente mais baixo (limite global).
    pub fn limited(&self, max_ma: f32) -> Adjust {
        let max = self.max_ma.map_or(max_ma, |m| m.min(max_ma));
        Adjust { max_ma: Some(max.max(0.0)), ..self.clone() }
    }

    /// `input` em 0..1. Preenche `out` com o valor que vai para o LED (com gama)
    /// e `display` com a cor para mostrar na tela (sem gama). Devolve a corrente
    /// estimada em mA, já depois do limite.
    pub fn apply(&self, input: &[[f32; 3]], out: &mut Vec<Color>, display: &mut Vec<Color>) -> f32 {
        let mut lin: Vec<[f32; 3]> = input
            .iter()
            .map(|&[r, g, b]| {
                let l = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                let s = |c: f32| (l + (c - l) * self.saturation).clamp(0.0, 1.0);
                [
                    s(r) * self.white[0] * self.brightness,
                    s(g) * self.white[1] * self.brightness,
                    s(b) * self.white[2] * self.brightness,
                ]
            })
            .collect();
        let pwm = |c: f32| c.powf(self.gamma);
        let per = MA_PER_CHANNEL * self.scale;
        let mut ma: f32 = lin.iter().map(|c| (pwm(c[0]) + pwm(c[1]) + pwm(c[2])) * per).sum();
        if let Some(max) = self.max_ma {
            if ma > max && ma > 0.0 {
                // Corta a corrente reduzindo o PWM por igual; volta para o espaço linear.
                let k = (max / ma).powf(1.0 / self.gamma);
                lin.iter_mut().flatten().for_each(|c| *c *= k);
                ma = max;
            }
        }
        let byte = |c: f32| (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        out.clear();
        display.clear();
        for c in &lin {
            out.push(Color { r: byte(pwm(c[0])), g: byte(pwm(c[1])), b: byte(pwm(c[2])) });
            display.push(Color { r: byte(c[0]), g: byte(c[1]), b: byte(c[2]) });
        }
        ma
    }
}

/// Multiplicadores RGB para uma temperatura de cor; 6500 K ≈ neutro.
/// Aproximação de Tanner Helland para o corpo negro.
fn white_balance(kelvin: u32) -> [f32; 3] {
    let k = kelvin_rgb(kelvin.clamp(1500, 12000) as f32);
    let n = kelvin_rgb(6500.0);
    let m = [k[0] / n[0], k[1] / n[1], k[2] / n[2]];
    let max = m[0].max(m[1]).max(m[2]);
    [m[0] / max, m[1] / max, m[2] / max]
}

fn kelvin_rgb(kelvin: f32) -> [f32; 3] {
    let t = kelvin / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.698_73 * (t - 60.0).powf(-0.133_204_76) };
    let g = if t <= 66.0 { 99.470_8 * t.ln() - 161.119_57 } else { 288.122_17 * (t - 60.0).powf(-0.075_514_85) };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.517_73 * (t - 10.0).ln() - 305.044_8
    };
    [r.clamp(1.0, 255.0), g.clamp(1.0, 255.0), b.clamp(1.0, 255.0)]
}

pub fn from_color(c: Color) -> [f32; 3] {
    [c.r as f32 / 255.0, c.g as f32 / 255.0, c.b as f32 / 255.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutro_nao_muda_a_cor() {
        let cfg = ColorCfg { brightness: 1.0, gamma: 1.0, saturation: 1.0, temperature: 6500, power_limit: false, ..Default::default() };
        let (mut out, mut disp) = (Vec::new(), Vec::new());
        Adjust::new(&cfg).apply(&[[1.0, 0.5, 0.0]], &mut out, &mut disp);
        assert_eq!(out[0], Color { r: 255, g: 128, b: 0 });
    }

    #[test]
    fn limite_de_corrente() {
        let cfg = ColorCfg { brightness: 1.0, gamma: 2.2, power_limit: true, max_current_ma: 600, ..Default::default() };
        let (mut out, mut disp) = (Vec::new(), Vec::new());
        // 20 LEDs brancos = 1200 mA sem limite.
        let ma = Adjust::new(&cfg).apply(&[[1.0; 3]; 20], &mut out, &mut disp);
        assert!(ma <= 600.0);
        let real: f32 = out.iter().map(|c| (c.r as f32 + c.g as f32 + c.b as f32) / 255.0 * MA_PER_CHANNEL).sum();
        assert!((real - 600.0).abs() < 20.0, "{real}");
    }

    #[test]
    fn saida_com_fans_em_paralelo_limita_a_corrente_real() {
        let mut fan = OutputCfg::fan(6);
        fan.parallel = 7;
        fan.max_current_ma = 1000;
        let color = ColorCfg { brightness: 1.0, gamma: 2.2, ..Default::default() };
        let (mut out, mut disp) = (Vec::new(), Vec::new());
        // 11 pixels brancos × 14 LEDs físicos ≈ 9,2 A sem limite.
        let ma = Adjust::for_output(&color, &fan).apply(&[[1.0; 3]; 11], &mut out, &mut disp);
        assert!(ma <= 1000.0 + 1e-3, "{ma}");
        let real: f32 = out.iter().map(|c| (c.r as f32 + c.g as f32 + c.b as f32) / 255.0 * MA_PER_CHANNEL * 14.0).sum();
        assert!((real - 1000.0).abs() < 60.0, "{real}");
        // Limite global por cima do limite da saída.
        let ma = Adjust::for_output(&color, &fan).limited(400.0).apply(&[[1.0; 3]; 11], &mut out, &mut disp);
        assert!(ma <= 400.0 + 1e-3);
    }
}
