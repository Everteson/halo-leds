//! Escuta o som que sai do PC (loopback do WASAPI: o mesmo áudio que vai para
//! a caixa ou o fone, sem microfone) e mede a energia dos graves e dos agudos.
//! Baseado em `app/src-tauri/src/audio.rs`; aqui também guardamos as últimas
//! amostras para calcular o espectro (FFT) a cada quadro.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::f32::consts::{PI, TAU};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Tamanho da FFT: 2048 amostras ≈ 43 ms a 48 kHz.
const FFT_SIZE: usize = 2048;

/// Energia (RMS) de cada faixa no último bloco de áudio.
#[derive(Debug, Clone, Copy, Default)]
pub struct Levels {
    pub bass: f32,
    pub treble: f32,
}

#[derive(Default)]
struct Shared {
    levels: Levels,
    samples: VecDeque<f32>,
    rate: f32,
}

#[derive(Default)]
pub struct Monitor {
    running: Arc<AtomicBool>,
    shared: Arc<Mutex<Shared>>,
}

impl Monitor {
    /// Começa a escutar a saída de som padrão. Não faz nada se já estiver escutando.
    pub fn start(&self) -> Result<(), String> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let running = self.running.clone();
        let shared = self.shared.clone();
        let (ready, result) = mpsc::channel();
        // O stream fica preso à thread que o criou; ela vive enquanto `running` for true.
        std::thread::spawn(move || {
            let stream = match open(shared.clone()) {
                Ok(s) => s,
                Err(e) => {
                    running.store(false, Ordering::SeqCst);
                    let _ = ready.send(Err(e));
                    return;
                }
            };
            let _ = ready.send(Ok(()));
            while running.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
            }
            drop(stream);
            *shared.lock().unwrap() = Shared::default();
        });
        result.recv().map_err(|_| "A captura de som parou inesperadamente".to_string())?
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub fn levels(&self) -> Levels {
        self.shared.lock().unwrap().levels
    }

    /// Magnitude em dB de `bands` faixas logarítmicas entre 40 Hz e 16 kHz.
    pub fn spectrum(&self, bands: usize, analyzer: &mut Analyzer) -> Vec<f32> {
        let rate = {
            let s = self.shared.lock().unwrap();
            analyzer.re.clear();
            analyzer.re.extend(s.samples.iter().copied());
            s.rate
        };
        analyzer.bands(rate, bands)
    }
}

fn open(shared: Arc<Mutex<Shared>>) -> Result<cpal::Stream, String> {
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("Nenhuma saída de som encontrada")?;
    let supported = device
        .default_output_config()
        .map_err(|e| format!("Não deu para ler a saída de som: {e}"))?;
    if supported.sample_format() != cpal::SampleFormat::F32 {
        return Err(format!("Formato de som não suportado: {:?}", supported.sample_format()));
    }
    let channels = supported.channels() as usize;
    let rate = supported.sample_rate() as f32;
    shared.lock().unwrap().rate = rate;
    let mut bands = Bands::new(rate);
    let stream = device
        .build_input_stream(
            supported.config(),
            move |data: &[f32], _: &cpal::InputCallbackInfo| {
                let l = bands.process(data, channels);
                let mut s = shared.lock().unwrap();
                s.levels = l;
                for frame in data.chunks_exact(channels.max(1)) {
                    s.samples.push_back(frame.iter().sum::<f32>() / frame.len() as f32);
                }
                let extra = s.samples.len().saturating_sub(FFT_SIZE);
                s.samples.drain(..extra);
            },
            |e| eprintln!("captura de som: {e}"),
            None,
        )
        .map_err(|e| format!("Não deu para capturar o som: {e}"))?;
    stream.play().map_err(|e| format!("Não deu para iniciar a captura de som: {e}"))?;
    Ok(stream)
}

/// Separa graves (< ~150 Hz) e agudos (> ~4 kHz) com filtros de um polo,
/// barato e suficiente para LEDs.
struct Bands {
    a_low: f32,
    a_high: f32,
    low: f32,
    below_high: f32,
}

impl Bands {
    fn new(rate: f32) -> Self {
        let coef = |fc: f32| 1.0 - (-TAU * fc / rate).exp();
        Bands { a_low: coef(150.0), a_high: coef(4000.0), low: 0.0, below_high: 0.0 }
    }

    fn process(&mut self, data: &[f32], channels: usize) -> Levels {
        let (mut b, mut t, mut n) = (0.0f32, 0.0f32, 0usize);
        for frame in data.chunks_exact(channels.max(1)) {
            let x = frame.iter().sum::<f32>() / frame.len() as f32;
            self.low += self.a_low * (x - self.low);
            self.below_high += self.a_high * (x - self.below_high);
            let treble = x - self.below_high;
            b += self.low * self.low;
            t += treble * treble;
            n += 1;
        }
        if n == 0 {
            return Levels::default();
        }
        let rms = |s: f32| (s / n as f32).sqrt();
        Levels { bass: rms(b), treble: rms(t) }
    }
}

/// Buffers reaproveitados entre quadros para a FFT.
#[derive(Default)]
pub struct Analyzer {
    re: Vec<f32>,
    im: Vec<f32>,
}

impl Analyzer {
    fn bands(&mut self, rate: f32, bands: usize) -> Vec<f32> {
        if self.re.len() < FFT_SIZE || rate <= 0.0 {
            return vec![-120.0; bands];
        }
        for (i, x) in self.re.iter_mut().enumerate() {
            *x *= 0.5 - 0.5 * (TAU * i as f32 / FFT_SIZE as f32).cos(); // janela de Hann
        }
        self.im.clear();
        self.im.resize(FFT_SIZE, 0.0);
        fft(&mut self.re, &mut self.im);
        let bin_hz = rate / FFT_SIZE as f32;
        let (lo, hi) = (40.0f32, 16000.0f32.min(rate / 2.0));
        (0..bands)
            .map(|b| {
                let f0 = lo * (hi / lo).powf(b as f32 / bands as f32);
                let f1 = lo * (hi / lo).powf((b + 1) as f32 / bands as f32);
                let k0 = ((f0 / bin_hz) as usize).max(1);
                let k1 = ((f1 / bin_hz) as usize).max(k0 + 1).min(FFT_SIZE / 2);
                let power = (k0..k1).map(|k| self.re[k] * self.re[k] + self.im[k] * self.im[k]).sum::<f32>()
                    / (k1 - k0).max(1) as f32;
                10.0 * (power + 1e-12).log10()
            })
            .collect()
    }
}

/// FFT complexa radix-2 in-place (n potência de 2).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let tr = re[b] * cr - im[b] * ci;
                let ti = re[b] * ci + im[b] * cr;
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let next = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = next;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_acha_o_tom() {
        let rate = 48000.0;
        let mut a = Analyzer::default();
        a.re = (0..FFT_SIZE).map(|i| (TAU * 900.0 * i as f32 / rate).sin()).collect();
        let b = a.bands(rate, 24);
        let peak = b.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).unwrap().0;
        // a faixa 12 vai de 800 Hz a ~1027 Hz
        assert_eq!(peak, 12, "{b:?}");
    }

    /// Toque algo no PC e rode: `cargo test escuta -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn escuta_a_saida_de_som() {
        let m = Monitor::default();
        m.start().expect("captura de som falhou");
        let mut a = Analyzer::default();
        for _ in 0..10 {
            std::thread::sleep(Duration::from_millis(300));
            let l = m.levels();
            let s = m.spectrum(8, &mut a);
            println!("graves {:.4}  agudos {:.4}  espectro {:?}", l.bass, l.treble, s.iter().map(|d| *d as i32).collect::<Vec<_>>());
        }
        m.stop();
    }
}
