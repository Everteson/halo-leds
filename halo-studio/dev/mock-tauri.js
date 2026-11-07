// Imita os comandos do backend para testar a interface no navegador.
// A "tela" é uma paisagem desenhada aqui mesmo, com o sol se mexendo.
// Começa como o usuário está hoje: uma fan (7 em paralelo) no D6 e uma fita no D9.
(() => {
  const C = (r, g, b) => ({ r, g, b });
  const GOLD = C(201, 165, 107), BLUE = C(46, 124, 214);
  const zones = () => ({ left: 11, top: 19, right: 11, bottom: 19, depth: 0.12, preset: 'full' });
  // Um anel interno (6 pixels) e uma moldura (5), como o mapa da fan Acegeek.
  const fanPoints = Array.from({ length: 11 }, (_, i) => {
    const inner = i < 6;
    const a = (inner ? i / 6 : (i - 6) / 5) * Math.PI * 2 + (inner ? 0 : 0.3);
    const r = inner ? 0.2 : 0.42;
    return [[+(0.5 + r * Math.sin(a)).toFixed(3), +(0.5 - r * Math.cos(a)).toFixed(3)]];
  });
  const output = (kind, pin, extra) => ({
    name: kind === 'fan' ? 'Cooler' : 'Fita', pin, kind, color_order: 2, parallel: 1,
    leds_per_pixel: kind === 'fan' ? 2 : 1, mode: kind === 'fan' ? 'effect' : 'screen',
    brightness: 1, power_limit: true, max_current_ma: 6000,
    leds: { count: kind === 'fan' ? 11 : 60, offset: 0, reverse: false, map: [], points: [], shape: kind, fans: 1 },
    zones: zones(),
    effect: { kind: 'orbit', colors: [GOLD, BLUE], speed: 4, size: 0.5, angle: 0, fan_phase: 0 },
    audio: { mode: kind === 'fan' ? 'space' : 'spectrum', sensitivity: 5, colors: [BLUE, GOLD] },
    sound: { enabled: false, amount: 0.6, floor: 0.4, band: 'bass', boost: 1.5, vivid: true },
    follow: true,
    cascade: 0,
    ...extra,
  });
  const defaults = () => ({
    version: 2,
    capture: { monitor: 0, fps: 60, exclude_taskbar: false, ignore_black_bars: true },
    color: { brightness: 0.8, gamma: 2.2, saturation: 1, temperature: 6500, smoothing: 0.5, power_limit: true, max_current_ma: 12000 },
    outputs: [output('fan', 6)],
    serial: { port: '', baud: 1000000, simulate: true },
    openrgb: { enabled: false, host: '127.0.0.1', port: 6742, devices: [] },
    app: { resume: true },
    groups: { fan: { phase: 0, overwrite: false }, strip: { phase: 0, overwrite: false } },
    state: { was_running: false, autostart_done: true },
  });
  let cfg = defaults();
  // Como o usuário montou: 3 grupos de cooler (2, 2 e 3 fans) e a fita de 113 LEDs no D9.
  const fanLeds = () => ({ count: 11, offset: 0, reverse: false, map: [], points: fanPoints, shape: 'fan', fans: 1 });
  cfg.outputs = [
    output('fan', 6, { name: 'Frente', parallel: 2, cascade: 0, leds: fanLeds() }),
    output('fan', 5, { name: 'Topo', parallel: 2, cascade: 1, leds: fanLeds() }),
    output('fan', 3, { name: 'Traseira', parallel: 3, cascade: 2, leds: fanLeds() }),
    output('strip', 9, { leds: { count: 113, offset: 0, reverse: false, map: [], points: [], shape: 'strip', fans: 1 } }),
  ];
  let running = false;
  let autostart = { enabled: true, command: '"C:\\Users\\voce\\AppData\\Local\\Halo Studio\\Halo Studio.exe" --autostart', current: true, dev: false };
  let firmware = { version: '', reply: '', outputs: '', pins: '', max: 0, ok: false };
  let seq = 0;
  const t0 = performance.now();
  const W = 320, H = 180;
  const cv = new OffscreenCanvas(W, H);
  const cx = cv.getContext('2d', { willReadFrequently: true });

  function paint(t) {
    const sky = cx.createLinearGradient(0, 0, 0, H);
    sky.addColorStop(0, '#1b2a55');
    sky.addColorStop(0.55, '#f08a4b');
    sky.addColorStop(1, '#2a1a2e');
    cx.fillStyle = sky;
    cx.fillRect(0, 0, W, H);
    cx.fillStyle = '#ffd38a';
    cx.beginPath();
    cx.arc(W * (0.3 + 0.4 * (0.5 + 0.5 * Math.sin(t * 0.4))), H * 0.42, 16, 0, 7);
    cx.fill();
    cx.fillStyle = '#26324a';
    cx.beginPath();
    cx.moveTo(0, 120);
    [[50, 70], [90, 100], [150, 40], [210, 95], [260, 60], [320, 110], [320, 180], [0, 180]].forEach(p => cx.lineTo(...p));
    cx.fill();
    cx.fillStyle = '#0c3b40';
    cx.fillRect(0, 140, W, 40);
    cx.fillStyle = '#103a1f';
    for (let x = 0; x < W; x += 14) {
      cx.beginPath();
      cx.moveTo(x, 140);
      cx.lineTo(x + 7, 112 - (x % 5) * 3);
      cx.lineTo(x + 14, 140);
      cx.fill();
    }
    return cx.getImageData(0, 0, W, H).data;
  }

  function layout(z, aspect) {
    const dy = Math.min(0.5, Math.max(0.02, z.depth)), dx = Math.min(0.5, dy / aspect);
    const r = [];
    for (let i = 0; i < z.left; i++) r.push([0, 1 - (i + 1) / z.left, dx, 1 - i / z.left]);
    for (let i = 0; i < z.top; i++) r.push([i / z.top, 0, (i + 1) / z.top, dy]);
    for (let i = 0; i < z.right; i++) r.push([1 - dx, i / z.right, 1, (i + 1) / z.right]);
    for (let i = 0; i < z.bottom; i++) r.push([1 - (i + 1) / z.bottom, 1 - dy, 1 - i / z.bottom, 1]);
    return r;
  }

  function sample(img, rects) {
    const out = new Uint8Array(rects.length * 3);
    rects.forEach((r, i) => {
      const s = [0, 0, 0];
      let k = 0;
      for (let y = Math.floor(r[1] * H); y < Math.ceil(r[3] * H); y++)
        for (let x = Math.floor(r[0] * W); x < Math.ceil(r[2] * W); x++, k++) for (let c = 0; c < 3; c++) s[c] += img[(y * W + x) * 4 + c];
      for (let c = 0; c < 3; c++) out[i * 3 + c] = s[c] / Math.max(1, k);
    });
    return out;
  }

  const rainbow = (h, v = 1) => [0, 2, 4].map(o => 255 * v * (0.5 + 0.5 * Math.cos(6.283 * (h + o / 6))));

  // Cores de uma saída num instante (aproximação: o backend de verdade faz melhor).
  function render(o, t, img, bands) {
    const n = o.leds.count;
    const out = [];
    const k = cfg.color.brightness * o.brightness;
    for (let i = 0; i < n; i++) {
      let c = [0, 0, 0];
      if (o.mode === 'screen' && img) {
        const rects = layout(o.zones, 16 / 9);
        const z = sample(img, [rects[Math.floor((i * rects.length) / n)]]);
        c = [z[0], z[1], z[2]];
      } else if ((o.mode === 'screen_color' || o.mode === 'screen_audio') && img) {
        c = [235, 130, 70].map(v => v * (o.mode === 'screen_audio' ? 0.3 + 0.7 * Math.abs(Math.sin(t * 3)) : 1));
      } else if (o.mode === 'audio') {
        c = rainbow(i / n, bands[Math.floor((i / n) * 23)] ?? 0.5);
      } else if (o.mode === 'effect') {
        c = rainbow((t * 0.2 + i / n) % 1);
      }
      // "Reagir ao som": batida de mentira a ~2 por segundo.
      const s = o.sound;
      const g = s?.enabled && (o.mode === 'screen' || o.mode === 'screen_color')
        ? 1 - s.amount * (1 - s.floor) * (1 - Math.max(0, Math.sin(t * 6)) ** 4) : 1;
      out.push(c.map(v => v * k * g));
    }
    // Limite da saída (estimativa de 20 mA por canal × LEDs físicos por pixel).
    const scale = Math.max(1, o.parallel) * Math.max(1, o.leds_per_pixel);
    let ma = out.reduce((s, c) => s + (c[0] + c[1] + c[2]) / 255 * 20 * scale, 0);
    if (o.power_limit && ma > o.max_current_ma) {
      const f = o.max_current_ma / ma;
      out.forEach(c => c.forEach((_, j) => (c[j] *= f)));
      ma = o.max_current_ma;
    }
    return { colors: out, ma };
  }

  function frame(withImage, idx) {
    const t = (performance.now() - t0) / 1000;
    const strip = idx != null ? cfg.outputs[idx] : null;
    const rects = strip ? layout(strip.zones, 16 / 9) : [];
    const needsScreen = running && cfg.outputs.some(o => ['screen', 'screen_color', 'screen_audio'].includes(o.mode));
    const img = needsScreen ? paint(t) : null;
    const zoneRgb = img && strip?.mode === 'screen' ? sample(img, rects) : new Uint8Array(rects.length * 3);
    const bands = running && cfg.outputs.some(o => o.mode === 'audio' || o.mode === 'screen_audio')
      ? Array.from({ length: 24 }, (_, b) => Math.max(0, Math.sin(t * 3 + b * 0.5) * 0.5 + 0.5 - b / 40)) : [];
    const outs = running ? cfg.outputs.map(o => render(o, t, img, bands)) : [];
    const leds = new Uint8Array(outs.reduce((s, o) => s + o.colors.length * 3, 0));
    let at = 0;
    outs.forEach(o => o.colors.forEach(c => (leds.set(c, at), (at += 3))));
    const currents = outs.map(o => o.ma);
    const w = withImage && img ? W : 0, h = withImage && img ? H : 0;
    const size = 20 + 16 + 16 + rects.length * 16 + w * h * 4 + rects.length * 3 + leds.length + bands.length * 4 + currents.length * 4;
    const buf = new ArrayBuffer(size);
    const dv = new DataView(buf);
    let o = 0;
    dv.setUint32(o, seq++, true); o += 4;
    const flags = (running ? 1 : 0) | (currents.length << 8);
    [w, h, rects.length, leds.length / 3, bands.length, flags].forEach(v => { dv.setUint16(o, v, true); o += 2; });
    const total = currents.reduce((s, v) => s + v, 0);
    [running ? 60 : 0, running ? 59.6 : 0, running ? 3.2 : 0, total, 0, 0, 1, 1, ...rects.flat()].forEach(v => { dv.setFloat32(o, v, true); o += 4; });
    const u8 = new Uint8Array(buf);
    if (w) { u8.set(img, o); o += img.length; }
    u8.set(zoneRgb, o); o += zoneRgb.length;
    u8.set(leds, o); o += leds.length;
    bands.forEach(v => { dv.setFloat32(o, v, true); o += 4; });
    currents.forEach(v => { dv.setFloat32(o, v, true); o += 4; });
    return buf;
  }

  // Firmware v2 de mentira: até 8 saídas, pinos 2–9, sem repetir, até 320 LEDs.
  function configure() {
    const outs = cfg.outputs.map(o => `${o.pin}:${o.leds.count}:${o.color_order}`).join(',');
    if (cfg.serial.simulate) return { version: 'simulado', reply: 'Simulação: a configuração não foi enviada', outputs: outs, pins: '', max: 0, ok: true };
    const pins = cfg.outputs.map(o => o.pin);
    const err = cfg.outputs.length < 1 || cfg.outputs.length > 8 ? 'n'
      : pins.some(p => p < 2 || p > 9) ? 'pino'
      : new Set(pins).size !== pins.length ? 'repetido'
      : cfg.outputs.reduce((s, o) => s + o.leds.count, 0) > 320 ? 'total' : null;
    const prev = firmware.outputs;
    const base = { version: 'v2', pins: '2-9', max: 320 };
    return err
      ? { ...base, reply: `Hlo ERR ${err}`, outputs: prev, ok: false }
      : { ...base, reply: 'Hlo OK', outputs: outs, ok: true };
  }

  const sleep = ms => new Promise(r => setTimeout(r, ms));
  const commands = {
    get_config: () => JSON.parse(JSON.stringify(cfg)),
    set_config: ({ config }) => { cfg = Object.keys(config).length ? JSON.parse(JSON.stringify(config)) : defaults(); },
    list_monitors: () => [
      { index: 0, name: 'DISPLAY1', width: 2560, height: 1440, primary: true },
      { index: 1, name: 'DISPLAY2', width: 1920, height: 1080, primary: false },
    ],
    list_ports: () => [
      { name: 'COM4', label: 'COM4 — USB-SERIAL CH340', arduino: true },
      { name: 'COM1', label: 'COM1 — Serial', arduino: false },
    ],
    start: async () => { await sleep(300); firmware = configure(); running = true; },
    stop: () => { running = false; },
    status: () => ({ running, error: null, config_path: 'C:\\Users\\voce\\AppData\\Roaming\\com.evert.halostudio\\config.json', simulate: cfg.serial.simulate, port: cfg.serial.port, firmware }),
    test_leds: async () => { await sleep(1200); firmware = configure(); return firmware; },
    apply_outputs: async () => { await sleep(500); firmware = configure(); return firmware; },
    openrgb_devices: () => [
      { index: 0, name: 'ASRock B450M Steel Legend', vendor: 'ASRock', kind: 0, leds: 6, streamable: false },
      { index: 1, name: 'Redragon Keyboard', vendor: 'Redragon', kind: 5, leds: 104, streamable: false },
      { index: 2, name: 'Logitech G502 X PLUS', vendor: 'Logitech', kind: 6, leds: 8, streamable: true },
    ],
    get_frame: ({ withImage, output }) => frame(withImage, output),
    mapper_open: async () => { await sleep(400); firmware = configure(); return firmware; },
    mapper_send: () => {},
    get_autostart: () => autostart,
    set_autostart: ({ enabled }) => (autostart = { ...autostart, enabled }),
    quit_app: () => {},
    mapper_close: () => {},
    effect_preview: ({ output }) => {
      const o = cfg.outputs[output];
      if (!o) return new ArrayBuffer(0);
      const { colors } = render({ ...o, mode: 'effect' }, (performance.now() - t0) / 1000, null, []);
      return new Uint8Array(colors.flat()).buffer;
    },
  };
  const ok = async () => {};
  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args = {}) => {
        if (!commands[cmd]) throw new Error(`comando desconhecido: ${cmd}`);
        return commands[cmd](args);
      },
    },
    window: { getCurrentWindow: () => ({ minimize: ok, toggleMaximize: ok, close: ok }) },
  };
})();
