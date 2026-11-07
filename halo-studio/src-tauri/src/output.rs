//! Saídas para os LEDs. Toda saída recebe a fita inteira já ajustada
//! (`Color` por LED) a cada quadro; cada backend decide como entregar.

use crate::config::Config;
use crate::openrgb::{Client, Color, Device};
use serde::Serialize;
use std::io::{Read, Write};
use std::time::{Duration, Instant};

const MODE_FLAG_AUTOMATIC_SAVE: u32 = 1 << 9;

pub trait LedOutput: Send {
    fn send(&mut self, leds: &[Color]) -> Result<(), String>;

    /// Diz ao Arduino quais pinos, quantos LEDs e qual ordem de cor cada
    /// saída tem. Só o firmware v2 entende; as outras saídas ignoram.
    fn configure(&mut self, _layout: &[(u8, u16, u8)]) -> Firmware {
        Firmware::default()
    }
}

/// O que o Arduino respondeu sobre as saídas, para mostrar na interface.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Firmware {
    /// "v2", "v1" (não respondeu ao Hlo?), "simulado" ou vazio (nada ainda).
    pub version: String,
    /// Resposta ao HloC: "Hlo OK", "Hlo ERR …" ou uma explicação.
    pub reply: String,
    /// Saídas que o Arduino diz ter: "6:11:2,9:120:2".
    pub outputs: String,
    /// Pinos aceitos ("2-9") e máximo de LEDs somando as saídas, se o firmware disse.
    pub pins: String,
    pub max: u16,
    pub ok: bool,
}

/// Pacote HloC: 'H' 'l' 'o' 'C' n [pino qtd_hi qtd_lo ordem]×n chk, com
/// chk = XOR dos bytes depois de 'C' ^ 0x55 (firmware/halo-adalight/PROTOCOLO.md).
pub fn hloc_packet(layout: &[(u8, u16, u8)]) -> Vec<u8> {
    let mut body = vec![layout.len() as u8];
    for &(pin, count, order) in layout {
        body.extend([pin, (count >> 8) as u8, count as u8, order]);
    }
    let chk = body.iter().fold(0x55u8, |a, b| a ^ b);
    let mut pkt = b"HloC".to_vec();
    pkt.extend(body);
    pkt.push(chk);
    pkt
}

/// Monta um quadro Adalight: 'A' 'd' 'a', contagem-1 (hi, lo), checksum e RGB.
pub fn adalight_frame(leds: &[Color], buf: &mut Vec<u8>) {
    let n = leds.len().clamp(1, 65536) as u32 - 1;
    let (hi, lo) = ((n >> 8) as u8, n as u8);
    buf.clear();
    buf.extend([b'A', b'd', b'a', hi, lo, hi ^ lo ^ 0x55]);
    for c in leds {
        buf.extend([c.r, c.g, c.b]);
    }
    if leds.is_empty() {
        buf.extend([0, 0, 0]);
    }
}

/// Byte que o firmware v2 manda depois de mostrar cada quadro.
const ACK: u8 = 0x06;
/// Quanto esperar o ACK antes de mandar o próximo quadro mesmo assim.
const ACK_TIMEOUT: Duration = Duration::from_millis(50);
const PORT_TIMEOUT: Duration = Duration::from_millis(500);

/// Controle de fluxo: durante o `show()` (~30 µs por LED) o Arduino perde os
/// bytes que chegam, então o próximo quadro só sai depois do ACK do anterior.
#[derive(Default)]
struct Flow {
    /// Há um quadro enviado esperando ACK.
    waiting: bool,
    /// O firmware manda ACK? `None` até descobrir; o v1 nunca manda.
    acks: Option<bool>,
    misses: u8,
}

/// Arduino com o firmware Adalight (firmware/halo-adalight).
pub struct Adalight {
    port: Box<dyn serialport::SerialPort>,
    buf: Vec<u8>,
    flow: Flow,
}

impl Adalight {
    /// Abrir a porta reinicia o Arduino UNO; esperamos o "Ada" que o firmware
    /// manda ao terminar o boot (o bootloader descartaria um quadro mandado
    /// antes disso), ou desistimos de esperar depois de ~3 s.
    pub fn open(name: &str, baud: u32) -> Result<Adalight, String> {
        if name.is_empty() {
            return Err("Escolha a porta COM do Arduino em Dispositivos".into());
        }
        let mut port = serialport::new(name, baud)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| open_error(name, e))?;
        let start = Instant::now();
        let mut seen = Vec::new();
        let mut chunk = [0u8; 64];
        while start.elapsed() < Duration::from_millis(3000) {
            if let Ok(n) = port.read(&mut chunk) {
                seen.extend_from_slice(&chunk[..n]);
                if seen.windows(3).any(|w| w == b"Ada") {
                    break;
                }
            }
        }
        let _ = port.clear(serialport::ClearBuffer::Input);
        port.set_timeout(PORT_TIMEOUT).map_err(|e| e.to_string())?;
        Ok(Adalight { port, buf: Vec::new(), flow: Flow::default() })
    }
}

/// No Windows, "acesso negado" (porta aberta por outro programa) e "não
/// encontrada" chegam como o mesmo erro; a lista de portas desempata.
fn open_error(name: &str, e: serialport::Error) -> String {
    if e.kind() != serialport::ErrorKind::NoDevice {
        return format!("Não deu para abrir {name}: {e}");
    }
    let present = serialport::available_ports().unwrap_or_default().iter().any(|p| p.port_name == name);
    if present {
        format!(
            "A {name} está em uso por outro programa (led-test, Arduino IDE, OpenRGB...). \
             Só um programa por vez pode abrir a porta: feche o outro e tente de novo."
        )
    } else {
        format!("A {name} não foi encontrada. O Arduino está ligado no USB?")
    }
}

impl Adalight {
    /// Lê linhas até achar uma que comece com `prefix` ou o tempo acabar.
    /// O firmware ocioso manda "Ada\n" a cada 1 s; essas linhas são puladas.
    fn read_line(&mut self, prefix: &str, timeout: Duration) -> Option<String> {
        let until = Instant::now() + timeout;
        let mut acc = Vec::new();
        let mut chunk = [0u8; 64];
        let _ = self.port.set_timeout(Duration::from_millis(50));
        let found = loop {
            if let Some(pos) = acc.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = acc.drain(..=pos).collect();
                let line = String::from_utf8_lossy(&line).trim().to_string();
                if line.starts_with(prefix) {
                    break Some(line);
                }
                continue;
            }
            if Instant::now() >= until {
                break None;
            }
            if let Ok(n) = self.port.read(&mut chunk) {
                // ACKs de quadros (0x06) não fazem parte das linhas de texto.
                acc.extend(chunk[..n].iter().filter(|&&b| b != ACK));
            }
        };
        let _ = self.port.set_timeout(PORT_TIMEOUT);
        found
    }

    /// Espera o ACK do quadro anterior, no máximo `ACK_TIMEOUT`. Se o
    /// firmware nunca mandou ACK em 3 quadros seguidos, para de esperar.
    fn wait_ack(&mut self) {
        self.flow.waiting = false;
        let until = Instant::now() + ACK_TIMEOUT;
        let mut chunk = [0u8; 64];
        let mut got = false;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() || self.port.set_timeout(left).is_err() {
                break;
            }
            match self.port.read(&mut chunk) {
                Ok(n) if chunk[..n].contains(&ACK) => {
                    got = true;
                    break;
                }
                Ok(_) => {} // "Ada\n" do firmware ocioso etc.
                Err(_) => break,
            }
        }
        // O timeout da porta também vale para a escrita do próximo quadro.
        let _ = self.port.set_timeout(PORT_TIMEOUT);
        if got {
            self.flow.acks = Some(true);
            self.flow.misses = 0;
        } else if self.flow.acks.is_none() {
            self.flow.misses += 1;
            if self.flow.misses >= 3 {
                self.flow.acks = Some(false);
            }
        }
    }

    /// `Hlo?` → "Hlo v2 max=320 pinos=2-9 saidas=6:11:2,9:120:2". O v1 não responde.
    fn query(&mut self) -> Option<String> {
        let _ = self.port.clear(serialport::ClearBuffer::Input);
        self.port.write_all(b"Hlo?").ok()?;
        self.read_line("Hlo v", Duration::from_millis(500))
    }
}

fn field<'a>(line: &'a str, key: &str) -> &'a str {
    line.split_whitespace().find_map(|w| w.strip_prefix(key)).unwrap_or("")
}

impl LedOutput for Adalight {
    fn send(&mut self, leds: &[Color]) -> Result<(), String> {
        if self.flow.waiting {
            self.wait_ack();
        }
        adalight_frame(leds, &mut self.buf);
        self.port.write_all(&self.buf).map_err(|e| format!("Arduino parou de responder: {e}"))?;
        self.flow.waiting = self.flow.acks != Some(false);
        Ok(())
    }

    fn configure(&mut self, layout: &[(u8, u16, u8)]) -> Firmware {
        // O `query` limpa a entrada: um ACK pendente se perderia ali.
        self.flow.waiting = false;
        let Some(info) = self.query() else {
            return Firmware {
                version: "v1".into(),
                reply: "Firmware v1: não entende saídas, manda tudo para o D6".into(),
                ok: layout.len() <= 1,
                ..Default::default()
            };
        };
        if layout.is_empty() || layout.len() > 8 {
            return Firmware {
                version: "v2".into(),
                reply: "O firmware aceita de 1 a 8 saídas".into(),
                outputs: field(&info, "saidas=").into(),
                pins: field(&info, "pinos=").into(),
                max: field(&info, "max=").parse().unwrap_or(0),
                ok: false,
            };
        }
        if self.port.write_all(&hloc_packet(layout)).is_err() {
            return Firmware { version: "v2".into(), reply: "Não deu para enviar a configuração".into(), ..Default::default() };
        }
        // Gravar na EEPROM leva alguns ms.
        let reply = self.read_line("Hlo ", Duration::from_millis(1500)).unwrap_or_else(|| "Sem resposta do Arduino".into());
        let now = self.query().unwrap_or(info);
        Firmware {
            version: "v2".into(),
            ok: reply == "Hlo OK",
            reply,
            outputs: field(&now, "saidas=").into(),
            pins: field(&now, "pinos=").into(),
            max: field(&now, "max=").parse().unwrap_or(0),
        }
    }
}

/// Simulação da saída serial: monta os quadros como se fosse enviar e só
/// registra no console de tempos em tempos. Não abre nenhuma porta.
pub struct SerialLog {
    buf: Vec<u8>,
    frames: u32,
    bytes: usize,
    since: Instant,
}

impl SerialLog {
    pub fn new() -> SerialLog {
        SerialLog { buf: Vec::new(), frames: 0, bytes: 0, since: Instant::now() }
    }
}

impl LedOutput for SerialLog {
    fn send(&mut self, leds: &[Color]) -> Result<(), String> {
        adalight_frame(leds, &mut self.buf);
        self.frames += 1;
        self.bytes += self.buf.len();
        let secs = self.since.elapsed().as_secs_f32();
        if secs >= 5.0 {
            eprintln!(
                "[serial simulado] {:.1} quadros/s, {} LEDs, {:.0} bytes/s, cabeçalho {:02X?}",
                self.frames as f32 / secs,
                leds.len(),
                self.bytes as f32 / secs,
                &self.buf[..6]
            );
            self.frames = 0;
            self.bytes = 0;
            self.since = Instant::now();
        }
        Ok(())
    }

    fn configure(&mut self, layout: &[(u8, u16, u8)]) -> Firmware {
        let outs: Vec<String> = layout.iter().map(|(p, n, o)| format!("{p}:{n}:{o}")).collect();
        eprintln!("[serial simulado] HloC {:02X?}", hloc_packet(layout));
        Firmware {
            version: "simulado".into(),
            reply: "Simulação: a configuração não foi enviada".into(),
            outputs: outs.join(","),
            ok: true,
            ..Default::default()
        }
    }
}

/// Dispositivos do OpenRGB em modo Direct. Cada um recebe a fita reamostrada
/// para o número de LEDs dele.
pub struct OpenRgbDirect {
    client: Client,
    targets: Vec<(u32, usize)>,
    colors: Vec<Color>,
}

/// Só dá para animar dispositivos com modo Direct sem gravação automática.
/// A placa ASRock Polychrome e o teclado EVision gravam na memória interna a
/// cada atualização; animar a 60 quadros/s desgastaria essa memória.
pub fn can_stream(device: &Device) -> bool {
    device
        .modes
        .iter()
        .any(|m| m.name.eq_ignore_ascii_case("direct") && m.flags & MODE_FLAG_AUTOMATIC_SAVE == 0)
}

impl OpenRgbDirect {
    pub fn open(host: &str, port: u16, wanted: &[u32]) -> Result<OpenRgbDirect, String> {
        let mut client = Client::connect(host, port, "Halo Studio")
            .map_err(|e| format!("OpenRGB não respondeu em {host}:{port}: {e}"))?;
        let devices = client.devices().map_err(|e| format!("OpenRGB: {e}"))?;
        let targets: Vec<(u32, usize)> = devices
            .iter()
            .filter(|d| wanted.contains(&d.index) && can_stream(d) && !d.leds.is_empty())
            .map(|d| (d.index, d.leds.len()))
            .collect();
        if targets.is_empty() {
            return Err("Nenhum dispositivo escolhido no OpenRGB aceita modo Direct sem gravar na memória".into());
        }
        for &(index, _) in &targets {
            client.set_custom_mode(index).map_err(|e| format!("OpenRGB: {e}"))?;
        }
        Ok(OpenRgbDirect { client, targets, colors: Vec::new() })
    }
}

impl LedOutput for OpenRgbDirect {
    fn send(&mut self, leds: &[Color]) -> Result<(), String> {
        if leds.is_empty() {
            return Ok(());
        }
        for &(index, n) in &self.targets {
            self.colors.clear();
            self.colors.extend((0..n).map(|k| leds[(k * leds.len() + leds.len() / 2) / n.max(1) % leds.len()]));
            self.client
                .update_leds(index, &self.colors)
                .map_err(|e| format!("Conexão com o OpenRGB caiu: {e}"))?;
        }
        Ok(())
    }
}

/// Abre as saídas ligadas na configuração.
pub fn open_all(cfg: &Config) -> Result<Vec<Box<dyn LedOutput>>, String> {
    let mut outs: Vec<Box<dyn LedOutput>> = Vec::new();
    if cfg.serial.simulate {
        outs.push(Box::new(SerialLog::new()));
    } else {
        outs.push(Box::new(Adalight::open(&cfg.serial.port, cfg.serial.baud)?));
    }
    if cfg.openrgb.enabled && !cfg.openrgb.devices.is_empty() {
        outs.push(Box::new(OpenRgbDirect::open(&cfg.openrgb.host, cfg.openrgb.port, &cfg.openrgb.devices)?));
    }
    Ok(outs)
}

#[derive(Debug, Serialize)]
pub struct PortInfo {
    pub name: String,
    pub label: String,
    /// Parece um Arduino (CH340, FTDI ou o USB do próprio Arduino).
    pub arduino: bool,
}

pub fn list_ports() -> Vec<PortInfo> {
    let mut ports: Vec<PortInfo> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let (label, arduino) = match &p.port_type {
                serialport::SerialPortType::UsbPort(u) => {
                    let product = u.product.clone().unwrap_or_else(|| "USB serial".into());
                    // O Windows repete a porta no nome: "USB-SERIAL CH340 (COM3)".
                    let product = product.trim_end_matches(&format!(" ({})", p.port_name)).to_string();
                    // 1A86 = WCH (CH340), 2341/2A03 = Arduino, 0403 = FTDI
                    (product, matches!(u.vid, 0x1A86 | 0x2341 | 0x2A03 | 0x0403))
                }
                serialport::SerialPortType::BluetoothPort => ("Bluetooth".into(), false),
                _ => ("Serial".into(), false),
            };
            PortInfo { label: format!("{} — {}", p.port_name, label), name: p.port_name, arduino }
        })
        .collect();
    ports.sort_by_key(|p| (!p.arduino, p.name.len(), p.name.clone()));
    ports
}

/// A porta configurada, se ela existe; senão a primeira com cara de Arduino
/// (CH340, Arduino, FTDI). A COM muda quando o Arduino troca de USB.
pub fn find_arduino(preferred: &str) -> Option<String> {
    let ports = list_ports();
    if ports.iter().any(|p| p.name == preferred) {
        return Some(preferred.to_string());
    }
    ports.into_iter().find(|p| p.arduino).map(|p| p.name)
}

/// Quadros do teste de uma saída: vermelho, verde e azul só nos LEDs dela
/// (`range` dentro do quadro de `total`), depois um ponto correndo do
/// primeiro LED (dourado) até o último. O resto do quadro fica apagado.
pub fn test_frames(total: usize, range: std::ops::Range<usize>) -> Vec<(Vec<Color>, Duration)> {
    let total = total.max(range.end).max(1);
    let dim = |r: u8, g: u8, b: u8| Color { r: r / 3, g: g / 3, b: b / 3 };
    let mut frames = Vec::new();
    for c in [dim(255, 0, 0), dim(0, 255, 0), dim(0, 0, 255)] {
        let mut f = vec![Color::default(); total];
        f[range.clone()].fill(c);
        frames.push((f, Duration::from_millis(600)));
    }
    let n = range.len().max(1);
    let step = Duration::from_millis((2400 / n as u64).clamp(8, 60));
    for i in 0..range.len() {
        let mut f = vec![Color::default(); total];
        let led = &mut f[range.clone()];
        for (k, c) in led.iter_mut().enumerate().take(i + 1) {
            let fade = 1.0 - (i - k) as f32 / 8.0;
            if fade > 0.0 {
                *c = Color { r: 0, g: (60.0 * fade) as u8, b: (200.0 * fade) as u8 };
            }
        }
        led[0] = dim(201, 165, 107);
        frames.push((f, step));
    }
    frames.push((vec![Color::default(); total], Duration::ZERO));
    frames
}

/// Abre a porta só durante o teste, configura as saídas e acende só a
/// escolhida. Devolve o que o firmware respondeu.
pub fn test_output(port: &str, baud: u32, layout: &[(u8, u16, u8)], range: std::ops::Range<usize>) -> Result<Firmware, String> {
    let mut out = Adalight::open(port, baud)?;
    let fw = out.configure(layout);
    let total = layout.iter().map(|l| l.1 as usize).sum();
    for (frame, hold) in test_frames(total, range) {
        let until = Instant::now() + hold;
        loop {
            out.send(&frame)?;
            if Instant::now() >= until {
                break;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
    }
    Ok(fw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cabecalho_adalight() {
        let mut buf = Vec::new();
        adalight_frame(&vec![Color { r: 1, g: 2, b: 3 }; 300], &mut buf);
        // 300 LEDs -> contagem 299 = 0x012B
        assert_eq!(&buf[..6], &[b'A', b'd', b'a', 0x01, 0x2B, 0x01 ^ 0x2B ^ 0x55]);
        assert_eq!(buf.len(), 6 + 900);
        assert_eq!(&buf[6..9], &[1, 2, 3]);
    }

    #[test]
    fn campos_da_resposta_do_hlo() {
        let line = "Hlo v2 max=320 pinos=2-9 saidas=6:11:2,9:300:2";
        assert_eq!(field(line, "max="), "320");
        assert_eq!(field(line, "pinos="), "2-9");
        assert_eq!(field(line, "saidas="), "6:11:2,9:300:2");
    }

    #[test]
    fn pacote_hloc_do_protocolo() {
        // Fan no D6 (11, GRB) e fita no D9 (120, GRB).
        let p = hloc_packet(&[(6, 11, 2), (9, 120, 2)]);
        let body = [2u8, 6, 0, 11, 2, 9, 0, 120, 2];
        assert_eq!(&p[..4], b"HloC");
        assert_eq!(&p[4..13], &body);
        assert_eq!(p[13], body.iter().fold(0x55, |a, b| a ^ b));
    }

    #[test]
    fn teste_acende_so_a_saida_escolhida() {
        let frames = test_frames(131, 11..131);
        for (f, _) in &frames {
            assert_eq!(f.len(), 131);
            assert!(f[..11].iter().all(|c| *c == Color::default()), "a fan não pode acender");
        }
        assert!(frames[0].0[11..].iter().all(|c| c.r > 0));
        assert_eq!(frames.last().unwrap().0.iter().filter(|c| **c != Color::default()).count(), 0);
    }

    #[test]
    fn porta_inexistente_tem_mensagem_clara() {
        let e = Adalight::open("COM99", 1_000_000).err().unwrap();
        assert!(e.contains("não foi encontrada"), "{e}");
    }
}
