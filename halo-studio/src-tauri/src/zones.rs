//! Zonas em volta da borda da tela: geometria, cor média de cada zona na
//! imagem reduzida e mapeamento zona → LED.

use crate::capture::Image;
use crate::config::{LedsCfg, ZonesCfg};

/// Retângulo normalizado (0..1) dentro da área útil da imagem: x0, y0, x1, y1.
pub type Rect = [f32; 4];

/// Zonas no sentido horário a partir do canto inferior esquerdo.
/// `aspect` = largura / altura da tela; as zonas laterais ficam com a mesma
/// espessura em pixels das de cima e de baixo.
pub fn layout(cfg: &ZonesCfg, aspect: f32) -> Vec<Rect> {
    let dy = cfg.depth.clamp(0.02, 0.5);
    let dx = (dy / aspect.max(0.1)).min(0.5);
    let mut rects = Vec::with_capacity(cfg.total());
    let n = cfg.left as f32;
    for i in 0..cfg.left {
        let i = i as f32;
        rects.push([0.0, 1.0 - (i + 1.0) / n, dx, 1.0 - i / n]);
    }
    let n = cfg.top as f32;
    for i in 0..cfg.top {
        let i = i as f32;
        rects.push([i / n, 0.0, (i + 1.0) / n, dy]);
    }
    let n = cfg.right as f32;
    for i in 0..cfg.right {
        let i = i as f32;
        rects.push([1.0 - dx, i / n, 1.0, (i + 1.0) / n]);
    }
    let n = cfg.bottom as f32;
    for i in 0..cfg.bottom {
        let i = i as f32;
        rects.push([1.0 - (i + 1.0) / n, 1.0 - dy, 1.0 - i / n, 1.0]);
    }
    rects
}

/// Área da imagem que conta como "tela": tira a barra de tarefas e, se
/// pedido, as faixas pretas de filmes. Normalizada 0..1.
pub fn content_area(img: &Image, work: Rect, black_bars: bool, last: Rect) -> Rect {
    if !black_bars || img.width == 0 {
        return work;
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let px = |x: usize, y: usize| {
        let p = &img.bgra[(y * w + x) * 4..];
        p[0].max(p[1]).max(p[2])
    };
    let (x0, y0) = ((work[0] * w as f32) as usize, (work[1] * h as f32) as usize);
    let (x1, y1) = (((work[2] * w as f32) as usize).min(w), ((work[3] * h as f32) as usize).min(h));
    if x1 <= x0 + 4 || y1 <= y0 + 4 {
        return work;
    }
    // Uma linha é "faixa" se quase todos os pixels dela forem quase pretos.
    const DARK: u8 = 18;
    let row_dark = |y: usize| (x0..x1).filter(|&x| px(x, y) > DARK).count() * 50 < (x1 - x0);
    let col_dark = |x: usize| (y0..y1).filter(|&y| px(x, y) > DARK).count() * 50 < (y1 - y0);
    let max_v = (y1 - y0) / 4;
    let max_h = (x1 - x0) / 4;
    let top = (y0..y0 + max_v).take_while(|&y| row_dark(y)).count();
    let bottom = (y1 - max_v..y1).rev().take_while(|&y| row_dark(y)).count();
    let left = (x0..x0 + max_h).take_while(|&x| col_dark(x)).count();
    let right = (x1 - max_h..x1).rev().take_while(|&x| col_dark(x)).count();
    // Cena toda escura: mantém o recorte anterior em vez de pular.
    if top >= max_v && bottom >= max_v {
        return last;
    }
    // Faixas reais são simétricas; se não forem, é só uma cena escura na borda.
    let v = top.min(bottom);
    let hz = left.min(right);
    let v = if v >= 2 { v } else { 0 };
    let hz = if hz >= 2 { hz } else { 0 };
    [
        (x0 + hz) as f32 / w as f32,
        (y0 + v) as f32 / h as f32,
        (x1 - hz) as f32 / w as f32,
        (y1 - v) as f32 / h as f32,
    ]
}

/// Cor média (0..1, RGB) de cada zona dentro de `area`.
pub fn sample(img: &Image, area: Rect, rects: &[Rect], out: &mut Vec<[f32; 3]>) {
    out.clear();
    let (w, h) = (img.width as f32, img.height as f32);
    let (ax, ay) = (area[0] * w, area[1] * h);
    let (aw, ah) = ((area[2] - area[0]) * w, (area[3] - area[1]) * h);
    for r in rects {
        let x0 = ((ax + r[0] * aw) as usize).min(img.width as usize - 1);
        let y0 = ((ay + r[1] * ah) as usize).min(img.height as usize - 1);
        let x1 = ((ax + r[2] * aw).ceil() as usize).clamp(x0 + 1, img.width as usize);
        let y1 = ((ay + r[3] * ah).ceil() as usize).clamp(y0 + 1, img.height as usize);
        let (mut b, mut g, mut rr) = (0u32, 0u32, 0u32);
        for y in y0..y1 {
            let row = &img.bgra[(y * img.width as usize + x0) * 4..(y * img.width as usize + x1) * 4];
            for p in row.chunks_exact(4) {
                b += p[0] as u32;
                g += p[1] as u32;
                rr += p[2] as u32;
            }
        }
        let n = ((x1 - x0) * (y1 - y0)) as f32 * 255.0;
        out.push([rr as f32 / n, g as f32 / n, b as f32 / n]);
    }
}

/// Cor dominante da área (0..1): média ponderada pela saturação, para os tons
/// vivos pesarem mais que cinzas e pretos, e com a saturação realçada.
pub fn dominant(img: &Image, area: Rect) -> [f32; 3] {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 {
        return [0.0; 3];
    }
    let x0 = ((area[0] * w as f32) as usize).min(w - 1);
    let y0 = ((area[1] * h as f32) as usize).min(h - 1);
    let x1 = ((area[2] * w as f32) as usize).clamp(x0 + 1, w);
    let y1 = ((area[3] * h as f32) as usize).clamp(y0 + 1, h);
    let (mut sum, mut total, mut light) = ([0.0f32; 3], 0.0f32, 0.0f32);
    let mut n = 0.0f32;
    // Um pixel a cada 2 em cada eixo basta para a média.
    for y in (y0..y1).step_by(2) {
        for x in (x0..x1).step_by(2) {
            let p = &img.bgra[(y * w + x) * 4..];
            let c = [p[2] as f32 / 255.0, p[1] as f32 / 255.0, p[0] as f32 / 255.0];
            let max = c[0].max(c[1]).max(c[2]);
            let chroma = max - c[0].min(c[1]).min(c[2]);
            let wgt = chroma * chroma * max + 0.002;
            for k in 0..3 {
                sum[k] += c[k] * wgt;
            }
            total += wgt;
            light += max;
            n += 1.0;
        }
    }
    let avg = sum.map(|v| v / total.max(1e-6));
    // A média ponderada puxa para os tons vivos, mas fica escura; devolve a
    // luminosidade da tela e realça a saturação (×1,6).
    let peak = avg[0].max(avg[1]).max(avg[2]).max(1e-4);
    let level = (light / n.max(1.0)).clamp(0.0, 1.0);
    let c = avg.map(|v| v / peak * level);
    let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    c.map(|v| (l + (v - l) * 1.6).clamp(0.0, 1.0))
}

/// Para cada LED, o índice da zona que ele mostra (`None` = apagado).
pub fn led_map(cfg: &LedsCfg, zones: usize) -> Vec<Option<usize>> {
    let count = cfg.count as usize;
    if cfg.map.len() == count {
        return cfg.map.iter().map(|&z| (z >= 0 && (z as usize) < zones).then_some(z as usize)).collect();
    }
    if zones == 0 {
        return vec![None; count];
    }
    (0..count)
        .map(|i| {
            let pos = if cfg.reverse { count - 1 - i } else { i };
            let z = pos * zones / count.max(1);
            Some((z as i64 + cfg.offset as i64).rem_euclid(zones as i64) as usize)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zonas_no_sentido_horario() {
        let cfg = ZonesCfg { left: 2, top: 3, right: 2, bottom: 3, depth: 0.1, preset: String::new() };
        let r = layout(&cfg, 16.0 / 9.0);
        assert_eq!(r.len(), 10);
        assert!(r[0][1] > r[1][1], "esquerda sobe");
        assert!(r[2][0] < r[3][0], "topo vai para a direita");
        assert!(r[5][1] < r[6][1], "direita desce");
        assert!(r[7][0] > r[8][0], "base volta para a esquerda");
    }

    #[test]
    fn mapeamento_com_offset_e_sentido() {
        let cfg = LedsCfg { count: 4, offset: 1, ..Default::default() };
        assert_eq!(led_map(&cfg, 4), vec![Some(1), Some(2), Some(3), Some(0)]);
        let cfg = LedsCfg { count: 4, reverse: true, ..Default::default() };
        assert_eq!(led_map(&cfg, 4), vec![Some(3), Some(2), Some(1), Some(0)]);
        let cfg = LedsCfg { count: 3, map: vec![2, -1, 0], ..Default::default() };
        assert_eq!(led_map(&cfg, 4), vec![Some(2), None, Some(0)]);
    }

    #[test]
    fn dominante_prefere_a_cor_viva() {
        // 3/4 cinza escuro e 1/4 vermelho vivo: a dominante é vermelha.
        let mut bgra = Vec::new();
        for _ in 0..4 {
            for x in 0..4 {
                bgra.extend(if x == 0 { [0, 0, 230, 255] } else { [60, 60, 60, 255] });
            }
        }
        let img = Image { width: 4, height: 4, bgra };
        let c = dominant(&img, [0.0, 0.0, 1.0, 1.0]);
        assert!(c[0] > 0.3 && c[0] > c[1] * 2.0 && c[0] > c[2] * 2.0, "{c:?}");
    }

    #[test]
    fn media_de_uma_imagem_dividida() {
        // 4×2: metade esquerda vermelha, direita azul (BGRA).
        let mut bgra = Vec::new();
        for _ in 0..2 {
            for x in 0..4 {
                bgra.extend(if x < 2 { [0, 0, 255, 255] } else { [255, 0, 0, 255] });
            }
        }
        let img = Image { width: 4, height: 2, bgra };
        let mut out = Vec::new();
        sample(&img, [0.0, 0.0, 1.0, 1.0], &[[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 1.0, 1.0]], &mut out);
        assert_eq!(out[0], [1.0, 0.0, 0.0]);
        assert_eq!(out[1], [0.0, 0.0, 1.0]);
    }
}
