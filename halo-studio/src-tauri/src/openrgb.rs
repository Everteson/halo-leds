//! Cliente do protocolo SDK do OpenRGB (TCP, porta 6742 por padrão).
//! Cópia enxuta de `app/src-tauri/src/openrgb.rs`: só lê os dispositivos e
//! envia cores LED a LED (modo Direct). Nada aqui grava modos na memória.
//!
//! Referência: Documentation/OpenRGBSDK.md no repositório do OpenRGB.
//! Negociamos no máximo a versão 5 do protocolo: é a última estável e a que
//! este parser conhece. Servidores mais novos respondem no formato pedido.

use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub const MAX_PROTOCOL: u32 = 5;

const REQUEST_CONTROLLER_COUNT: u32 = 0;
const REQUEST_CONTROLLER_DATA: u32 = 1;
const REQUEST_PROTOCOL_VERSION: u32 = 40;
const SET_CLIENT_NAME: u32 = 50;
const UPDATELEDS: u32 = 1050;
const SETCUSTOMMODE: u32 = 1100;

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mode {
    pub name: String,
    pub value: i32,
    pub flags: u32,
    pub speed_min: u32,
    pub speed_max: u32,
    pub brightness_min: u32,
    pub brightness_max: u32,
    pub colors_min: u32,
    pub colors_max: u32,
    pub speed: u32,
    pub brightness: u32,
    pub direction: u32,
    pub color_mode: u32,
    pub colors: Vec<Color>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Matrix {
    pub height: u32,
    pub width: u32,
    pub map: Vec<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Segment {
    pub name: String,
    pub kind: i32,
    pub start: u32,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Zone {
    pub name: String,
    pub kind: i32,
    pub leds_min: u32,
    pub leds_max: u32,
    pub leds_count: u32,
    pub matrix: Option<Matrix>,
    pub segments: Vec<Segment>,
    pub flags: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct Device {
    pub index: u32,
    pub kind: i32,
    pub name: String,
    pub vendor: String,
    pub description: String,
    pub version: String,
    pub serial: String,
    pub location: String,
    pub active_mode: i32,
    pub modes: Vec<Mode>,
    pub zones: Vec<Zone>,
    pub leds: Vec<String>,
    pub colors: Vec<Color>,
    pub flags: u32,
}

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> io::Result<&'a [u8]> {
        if self.pos + n > self.buf.len() {
            return Err(bad("pacote do OpenRGB truncado"));
        }
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn i32(&mut self) -> io::Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> io::Result<String> {
        let n = self.u16()? as usize;
        let b = self.take(n)?;
        let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
        Ok(String::from_utf8_lossy(&b[..end]).into_owned())
    }
    fn color(&mut self) -> io::Result<Color> {
        let b = self.take(4)?;
        Ok(Color { r: b[0], g: b[1], b: b[2] })
    }
    fn colors(&mut self) -> io::Result<Vec<Color>> {
        let n = self.u16()?;
        (0..n).map(|_| self.color()).collect()
    }
}

fn read_mode(r: &mut Reader, p: u32) -> io::Result<Mode> {
    let name = r.string()?;
    let value = r.i32()?;
    let flags = r.u32()?;
    let speed_min = r.u32()?;
    let speed_max = r.u32()?;
    let (brightness_min, brightness_max) = if p >= 3 { (r.u32()?, r.u32()?) } else { (0, 0) };
    let colors_min = r.u32()?;
    let colors_max = r.u32()?;
    let speed = r.u32()?;
    let brightness = if p >= 3 { r.u32()? } else { 0 };
    let direction = r.u32()?;
    let color_mode = r.u32()?;
    let colors = r.colors()?;
    Ok(Mode {
        name,
        value,
        flags,
        speed_min,
        speed_max,
        brightness_min,
        brightness_max,
        colors_min,
        colors_max,
        speed,
        brightness,
        direction,
        color_mode,
        colors,
    })
}

fn read_matrix(r: &mut Reader) -> io::Result<Option<Matrix>> {
    if r.u16()? == 0 {
        return Ok(None);
    }
    let height = r.u32()?;
    let width = r.u32()?;
    let map = (0..height * width).map(|_| r.u32()).collect::<io::Result<_>>()?;
    Ok(Some(Matrix { height, width, map }))
}

fn read_zone(r: &mut Reader, p: u32) -> io::Result<Zone> {
    let name = r.string()?;
    let kind = r.i32()?;
    let leds_min = r.u32()?;
    let leds_max = r.u32()?;
    let leds_count = r.u32()?;
    let matrix = read_matrix(r)?;
    let mut segments = Vec::new();
    if p >= 4 {
        for _ in 0..r.u16()? {
            segments.push(Segment {
                name: r.string()?,
                kind: r.i32()?,
                start: r.u32()?,
                count: r.u32()?,
            });
        }
    }
    let flags = if p >= 5 { r.u32()? } else { 0 };
    Ok(Zone { name, kind, leds_min, leds_max, leds_count, matrix, segments, flags })
}

fn read_device(index: u32, data: &[u8], p: u32) -> io::Result<Device> {
    let mut r = Reader { buf: data, pos: 0 };
    r.u32()?; // data_size
    let kind = r.i32()?;
    let name = r.string()?;
    let vendor = if p >= 1 { r.string()? } else { String::new() };
    let description = r.string()?;
    let version = r.string()?;
    let serial = r.string()?;
    let location = r.string()?;
    let num_modes = r.u16()?;
    let active_mode = r.i32()?;
    let modes = (0..num_modes).map(|_| read_mode(&mut r, p)).collect::<io::Result<_>>()?;
    let num_zones = r.u16()?;
    let zones = (0..num_zones).map(|_| read_zone(&mut r, p)).collect::<io::Result<_>>()?;
    let mut leds = Vec::new();
    for _ in 0..r.u16()? {
        let led_name = r.string()?;
        r.u32()?; // led_value
        leds.push(led_name);
    }
    let colors = r.colors()?;
    let mut flags = 0;
    if p >= 5 {
        // Nomes de exibição dos LEDs substituem os nomes internos quando existem.
        let n = r.u16()? as usize;
        for i in 0..n {
            let display = r.string()?;
            if let Some(led) = leds.get_mut(i) {
                if !display.is_empty() {
                    *led = display;
                }
            }
        }
        flags = r.u32()?;
    }
    Ok(Device {
        index,
        kind,
        name,
        vendor,
        description,
        version,
        serial,
        location,
        active_mode,
        modes,
        zones,
        leds,
        colors,
        flags,
    })
}

fn put_colors(out: &mut Vec<u8>, colors: &[Color]) {
    out.extend((colors.len() as u16).to_le_bytes());
    for c in colors {
        out.extend([c.r, c.g, c.b, 0]);
    }
}

/// Prefixa o corpo com `data_size`, que no protocolo inclui o próprio campo.
fn sized(body: Vec<u8>) -> Vec<u8> {
    let mut out = ((body.len() + 4) as u32).to_le_bytes().to_vec();
    out.extend(body);
    out
}

pub struct Client {
    stream: TcpStream,
    pub protocol: u32,
}

impl Client {
    pub fn connect(host: &str, port: u16, name: &str) -> io::Result<Client> {
        let addr = (host, port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| bad("endereço inválido"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut c = Client { stream, protocol: 0 };

        c.send(0, REQUEST_PROTOCOL_VERSION, &MAX_PROTOCOL.to_le_bytes())?;
        c.protocol = match c.recv_expect(REQUEST_PROTOCOL_VERSION) {
            Ok(d) if d.len() >= 4 => u32::from_le_bytes(d[..4].try_into().unwrap()).min(MAX_PROTOCOL),
            Ok(_) => 0,
            // Servidores v0 não respondem a este pedido.
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => 0,
            Err(e) => return Err(e),
        };
        c.stream.set_read_timeout(Some(Duration::from_secs(15)))?;

        let mut n = name.as_bytes().to_vec();
        n.push(0);
        c.send(0, SET_CLIENT_NAME, &n)?;
        Ok(c)
    }

    fn send(&mut self, dev: u32, id: u32, payload: &[u8]) -> io::Result<()> {
        let mut pkt = Vec::with_capacity(16 + payload.len());
        pkt.extend(b"ORGB");
        pkt.extend(dev.to_le_bytes());
        pkt.extend(id.to_le_bytes());
        pkt.extend((payload.len() as u32).to_le_bytes());
        pkt.extend(payload);
        self.stream.write_all(&pkt)
    }

    fn recv(&mut self) -> io::Result<(u32, Vec<u8>)> {
        let mut head = [0u8; 16];
        self.stream.read_exact(&mut head)?;
        if &head[..4] != b"ORGB" {
            return Err(bad("resposta inválida do OpenRGB"));
        }
        let id = u32::from_le_bytes(head[8..12].try_into().unwrap());
        let size = u32::from_le_bytes(head[12..16].try_into().unwrap()) as usize;
        if size > 64 * 1024 * 1024 {
            return Err(bad("pacote grande demais"));
        }
        let mut data = vec![0u8; size];
        self.stream.read_exact(&mut data)?;
        Ok((id, data))
    }

    /// Lê até chegar a resposta `id`, descartando avisos como DEVICE_LIST_UPDATED.
    fn recv_expect(&mut self, id: u32) -> io::Result<Vec<u8>> {
        loop {
            let (got, data) = self.recv()?;
            if got == id {
                return Ok(data);
            }
        }
    }

    pub fn controller_count(&mut self) -> io::Result<u32> {
        self.send(0, REQUEST_CONTROLLER_COUNT, &[])?;
        let d = self.recv_expect(REQUEST_CONTROLLER_COUNT)?;
        Reader { buf: &d, pos: 0 }.u32()
    }

    pub fn controller(&mut self, index: u32) -> io::Result<Device> {
        let payload = if self.protocol >= 1 { self.protocol.to_le_bytes().to_vec() } else { Vec::new() };
        self.send(index, REQUEST_CONTROLLER_DATA, &payload)?;
        let d = self.recv_expect(REQUEST_CONTROLLER_DATA)?;
        read_device(index, &d, self.protocol)
    }

    pub fn devices(&mut self) -> io::Result<Vec<Device>> {
        let n = self.controller_count()?;
        (0..n).map(|i| self.controller(i)).collect()
    }

    pub fn update_leds(&mut self, index: u32, colors: &[Color]) -> io::Result<()> {
        let mut body = Vec::new();
        put_colors(&mut body, colors);
        self.send(index, UPDATELEDS, &sized(body))
    }

    /// Coloca o dispositivo no modo "Direct" (controle LED a LED).
    pub fn set_custom_mode(&mut self, index: u32) -> io::Result<()> {
        self.send(index, SETCUSTOMMODE, &[])
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// Precisa de um OpenRGB rodando com --server: `cargo test -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lista_dispositivos_do_servidor_local() {
        let mut c = Client::connect("127.0.0.1", 6742, "teste").expect("OpenRGB não está rodando");
        println!("protocolo {}", c.protocol);
        for d in c.devices().unwrap() {
            println!("#{} {} [{}] tipo={} leds={} modo={}", d.index, d.name, d.vendor, d.kind, d.leds.len(), d.active_mode);
            for z in &d.zones {
                println!("    zona {} ({} LEDs, tipo {})", z.name, z.leds_count, z.kind);
            }
            for m in &d.modes {
                println!("    modo {} flags={:#x} cores={}..{} color_mode={}", m.name, m.flags, m.colors_min, m.colors_max, m.color_mode);
            }
        }
    }
}
