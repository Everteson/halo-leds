//! Captura de tela com DXGI Desktop Duplication.
//!
//! O quadro do monitor nunca vem inteiro para a CPU: ele é copiado para uma
//! textura com mipmaps na própria GPU, `GenerateMips` faz a média em caixas
//! 2×2 várias vezes, e só o nível pequeno (~200–400 px de largura) é lido.
//! Isso custa uma cópia na GPU e ~200 KB de leitura por quadro.

use serde::Serialize;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITORINFO};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// Largura máxima da imagem reduzida usada para as zonas e a prévia.
const MAX_SMALL_WIDTH: u32 = 400;

#[derive(Debug, Clone, Serialize)]
pub struct MonitorInfo {
    pub index: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

pub enum Error {
    /// A duplicação ficou inválida (troca de resolução, UAC, jogo em tela cheia
    /// exclusiva...). Basta recriar o `Duplicator`.
    Lost,
    Other(String),
}

fn outputs() -> windows::core::Result<Vec<(IDXGIAdapter1, IDXGIOutput)>> {
    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1()? };
    let mut list = Vec::new();
    let mut a = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(a) } {
        let mut o = 0;
        while let Ok(output) = unsafe { adapter.EnumOutputs(o) } {
            list.push((adapter.clone(), output));
            o += 1;
        }
        a += 1;
    }
    Ok(list)
}

pub fn list_monitors() -> Result<Vec<MonitorInfo>, String> {
    let list = outputs().map_err(|e| format!("Não deu para listar os monitores: {e}"))?;
    Ok(list
        .iter()
        .enumerate()
        .filter_map(|(i, (_, output))| {
            let d = unsafe { output.GetDesc() }.ok()?;
            let r = d.DesktopCoordinates;
            let len = d.DeviceName.iter().position(|&c| c == 0).unwrap_or(d.DeviceName.len());
            Some(MonitorInfo {
                index: i as u32,
                name: String::from_utf16_lossy(&d.DeviceName[..len]).trim_start_matches(r"\\.\").to_string(),
                width: (r.right - r.left) as u32,
                height: (r.bottom - r.top) as u32,
                primary: r.left == 0 && r.top == 0,
            })
        })
        .collect())
}

pub struct Duplicator {
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    dup: IDXGIOutputDuplication,
    /// Criadas no primeiro quadro, com o tamanho real da textura do desktop.
    textures: Option<Textures>,
    /// Área útil (sem a barra de tarefas), normalizada 0..1: x0, y0, x1, y1.
    pub work_area: [f32; 4],
}

struct Textures {
    full_w: u32,
    full_h: u32,
    level: u32,
    mips: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    staging: ID3D11Texture2D,
    width: u32,
    height: u32,
}

/// Imagem reduzida em BGRA, linhas sem preenchimento.
#[derive(Default)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

fn other(what: &str) -> impl Fn(windows::core::Error) -> Error + '_ {
    move |e| Error::Other(format!("{what}: {e}"))
}

impl Duplicator {
    pub fn new(monitor: u32) -> Result<Duplicator, String> {
        let list = outputs().map_err(|e| format!("Não deu para listar os monitores: {e}"))?;
        let (adapter, output) = list
            .into_iter()
            .nth(monitor as usize)
            .ok_or_else(|| format!("Monitor {} não encontrado", monitor + 1))?;

        let mut device = None;
        let mut ctx = None;
        unsafe {
            D3D11CreateDevice(
                &adapter.cast::<IDXGIAdapter>().map_err(|e| e.to_string())?,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut ctx),
            )
        }
        .map_err(|e| format!("Não deu para abrir a placa de vídeo: {e}"))?;
        let device = device.ok_or("D3D11 não devolveu o dispositivo")?;
        let ctx = ctx.ok_or("D3D11 não devolveu o contexto")?;

        // DuplicateOutput1 converte telas HDR/10 bits para BGRA 8 bits; nem todo
        // sistema aceita, então caímos para a versão antiga.
        let dup = unsafe {
            output
                .cast::<IDXGIOutput5>()
                .and_then(|o5| o5.DuplicateOutput1(&device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]))
                .or_else(|_| output.cast::<IDXGIOutput1>().and_then(|o1| o1.DuplicateOutput(&device)))
        }
        .map_err(|e| format!("Não deu para capturar o monitor {}: {e}", monitor + 1))?;

        let work_area = unsafe { output.GetDesc() }.map(|d| work_area(&d)).unwrap_or([0.0, 0.0, 1.0, 1.0]);
        Ok(Duplicator { device, ctx, dup, textures: None, work_area })
    }

    /// Espera até `timeout_ms` por um quadro novo e grava a versão reduzida em
    /// `img`. Devolve o instante (QPC) em que o quadro apareceu na tela, ou
    /// `None` se a tela não mudou nesse tempo.
    pub fn grab(&mut self, timeout_ms: u32, img: &mut Image) -> Result<Option<i64>, Error> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;
        match unsafe { self.dup.AcquireNextFrame(timeout_ms, &mut info, &mut resource) } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(None),
            Err(_) => return Err(Error::Lost),
        }
        // Só o ponteiro do mouse mexeu: a imagem é a mesma.
        if info.LastPresentTime == 0 {
            unsafe { self.dup.ReleaseFrame() }.map_err(|_| Error::Lost)?;
            return Ok(None);
        }
        let copied = resource
            .ok_or_else(|| Error::Other("quadro vazio".into()))
            .and_then(|r| r.cast::<ID3D11Texture2D>().map_err(other("textura do quadro")))
            .and_then(|frame| self.copy_frame(&frame));
        unsafe { self.dup.ReleaseFrame() }.map_err(|_| Error::Lost)?;
        copied?;
        self.read_small(img)?;
        Ok(Some(info.LastPresentTime))
    }

    fn copy_frame(&mut self, frame: &ID3D11Texture2D) -> Result<(), Error> {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { frame.GetDesc(&mut desc) };
        if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            return Err(Error::Other(format!("formato de tela não suportado ({})", desc.Format.0)));
        }
        let stale = self.textures.as_ref().is_none_or(|t| t.full_w != desc.Width || t.full_h != desc.Height);
        if stale {
            self.textures = Some(self.make_textures(desc.Width, desc.Height).map_err(other("texturas da captura"))?);
        }
        let t = self.textures.as_ref().unwrap();
        unsafe {
            self.ctx.CopySubresourceRegion(&t.mips, 0, 0, 0, 0, frame, 0, None);
            self.ctx.GenerateMips(&t.srv);
            self.ctx.CopySubresourceRegion(&t.staging, 0, 0, 0, 0, &t.mips, t.level, None);
        }
        Ok(())
    }

    fn make_textures(&self, full_w: u32, full_h: u32) -> windows::core::Result<Textures> {
        let mut level = 0;
        while (full_w >> level) > MAX_SMALL_WIDTH {
            level += 1;
        }
        let (width, height) = ((full_w >> level).max(1), (full_h >> level).max(1));
        let base = D3D11_TEXTURE2D_DESC {
            Width: full_w,
            Height: full_h,
            MipLevels: level + 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_GENERATE_MIPS.0 as u32,
        };
        let staging_desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
            ..base
        };
        let mut mips = None;
        let mut staging = None;
        let mut srv = None;
        unsafe {
            self.device.CreateTexture2D(&base, None, Some(&mut mips))?;
            let mips = mips.as_ref().unwrap();
            self.device.CreateShaderResourceView(mips, None, Some(&mut srv))?;
            self.device.CreateTexture2D(&staging_desc, None, Some(&mut staging))?;
        }
        Ok(Textures {
            full_w,
            full_h,
            level,
            mips: mips.unwrap(),
            srv: srv.unwrap(),
            staging: staging.unwrap(),
            width,
            height,
        })
    }

    fn read_small(&self, img: &mut Image) -> Result<(), Error> {
        let t = self.textures.as_ref().ok_or_else(|| Error::Other("sem textura".into()))?;
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        unsafe { self.ctx.Map(&t.staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)) }.map_err(other("leitura da GPU"))?;
        let row = (t.width * 4) as usize;
        img.width = t.width;
        img.height = t.height;
        img.bgra.resize(row * t.height as usize, 0);
        for y in 0..t.height as usize {
            // SAFETY: a textura mapeada tem `height` linhas de `RowPitch` bytes.
            let src = unsafe {
                std::slice::from_raw_parts((mapped.pData as *const u8).add(y * mapped.RowPitch as usize), row)
            };
            img.bgra[y * row..(y + 1) * row].copy_from_slice(src);
        }
        unsafe { self.ctx.Unmap(&t.staging, 0) };
        Ok(())
    }
}

impl Drop for Duplicator {
    fn drop(&mut self) {
        // Garante que nenhum quadro fique preso se a thread sair no meio.
        let _ = unsafe { self.dup.ReleaseFrame() };
    }
}

/// Milissegundos desde o instante `qpc` (mesmo relógio do `LastPresentTime`).
pub fn ms_since(qpc: i64) -> f32 {
    let (mut now, mut freq) = (0i64, 0i64);
    unsafe {
        let _ = QueryPerformanceCounter(&mut now);
        let _ = QueryPerformanceFrequency(&mut freq);
    }
    (now - qpc) as f32 * 1000.0 / freq.max(1) as f32
}

/// Área de trabalho (tela menos a barra de tarefas) relativa ao monitor.
fn work_area(d: &DXGI_OUTPUT_DESC) -> [f32; 4] {
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !unsafe { GetMonitorInfoW(d.Monitor, &mut mi) }.as_bool() {
        return [0.0, 0.0, 1.0, 1.0];
    }
    let (m, w) = (mi.rcMonitor, mi.rcWork);
    let (mw, mh) = ((m.right - m.left).max(1) as f32, (m.bottom - m.top).max(1) as f32);
    [
        (w.left - m.left) as f32 / mw,
        (w.top - m.top) as f32 / mh,
        (w.right - m.left) as f32 / mw,
        (w.bottom - m.top) as f32 / mh,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// `cargo test captura -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn captura_do_monitor_principal() {
        for m in list_monitors().unwrap() {
            println!("{m:?}");
        }
        let mut d = Duplicator::new(0).unwrap();
        let mut img = Image::default();
        let start = Instant::now();
        let mut frames = 0;
        while start.elapsed().as_secs_f32() < 2.0 {
            match d.grab(16, &mut img) {
                Ok(Some(_)) => frames += 1,
                Ok(None) => {}
                Err(Error::Lost) => panic!("perdeu a captura"),
                Err(Error::Other(e)) => panic!("{e}"),
            }
        }
        println!("{frames} quadros novos em 2 s, imagem {}×{}", img.width, img.height);
    }
}
