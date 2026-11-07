// Interface do Halo Studio. Fala com o backend em Rust pelos comandos do Tauri.
const T = window.__TAURI__;
const invoke = T.core.invoke;
const $ = (s, el = document) => el.querySelector(s);
const $$ = (s, el = document) => [...el.querySelectorAll(s)];

let cfg = null;
let status = {};
let view = 'tela';
let monitors = [];
let lastError = null;
let lastFrame = null;

// Saída escolhida em cada tela. Os campos com data-bind="@.algo" editam
// cfg.outputs[pick[tela]].algo. -1 = nenhuma saída do tipo da aba.
const pick = { tela: 0, audio: 0, efeitos: 0, dispositivos: 0 };
// Aba de cada tela: Cooler (CPU) = fans, Fita (Monitor) = fitas.
const tab = { tela: 'strip', audio: 'fan', efeitos: 'fan' };
const TAB_NAMES = { fan: 'Cooler (CPU)', strip: 'Fita (Monitor)' };

const SIZE_LABELS = {
  comet: 'Rastro', orbit: 'Rastro', radar: 'Rastro', ripples: 'Largura', scanner: 'Largura',
  seesaw: 'Suavidade', yinyang: 'Suavidade', fire: 'Intensidade', aurora: 'Contraste',
  twinkle: 'Densidade', heartbeat: 'Duração', spiral: 'Torção', kaleido: 'Espelhos',
};
const MODE_NAMES = {
  screen: 'Tela (zonas)', screen_color: 'Cor da tela', effect: 'Efeito',
  audio: 'Som', screen_audio: 'Tela + Som', off: 'Desligada',
};
const KIND_MODES = {
  fan: ['effect', 'screen_color', 'audio', 'screen_audio', 'off'],
  strip: ['screen', 'effect', 'audio', 'off'],
};
const KIND_NAMES = { fan: 'Cooler', strip: 'Fita' };
const ORDER_NAMES = ['RGB', 'RBG', 'GRB', 'GBR', 'BRG', 'BGR'];
const MAX_OUTPUTS = 8;
const MA_PER_CHANNEL = 20;
// Tempo que o Arduino leva para mostrar cada LED (show() do FastLED).
const SHOW_US_PER_LED = 30;

// Pinos e máximo de LEDs: o que o firmware disse no Hlo? ("pinos=2-9 max=320"),
// ou o padrão do firmware v2 atual.
function pinRange() {
  const m = /^(\d+)-(\d+)$/.exec(status.firmware?.pins || '');
  const [a, b] = m ? [+m[1], +m[2]] : [2, 9];
  return Array.from({ length: b - a + 1 }, (_, i) => a + i);
}
const maxLeds = () => status.firmware?.max || 320;
const MODE_HINTS = {
  screen: 'Cada LED mostra a zona da borda da tela que fica atrás dele. Dá para o brilho reagir ao som (abaixo).',
  screen_color: 'A fan toda na cor dominante da tela, mudando devagar (~0,4 s) e ignorando faixas pretas.',
  effect: 'Escolha o efeito na tela Efeitos.',
  audio: 'Escolha a visualização na tela Áudio.',
  screen_audio: 'A cor da tela, com o brilho puxado pelo som. Ajuste a força do grave abaixo.',
  off: 'Esta saída fica apagada.',
};

// ---------------------------------------------------------------- avisos
function toast(msg, kind = '') {
  const el = document.createElement('div');
  el.className = `toast ${kind}`;
  el.textContent = msg;
  $('#toasts').append(el);
  setTimeout(() => el.remove(), kind === 'err' ? 7000 : 3500);
}
const fail = e => toast(String(e), 'err');
const fmtA = ma => (ma >= 1000 ? `${(ma / 1000).toFixed(ma >= 10000 ? 1 : 2).replace('.', ',')} A` : `${Math.round(ma)} mA`);

// ---------------------------------------------------------------- janela
const win = T.window?.getCurrentWindow?.();
$('#win-min').onclick = () => win?.minimize();
$('#win-max').onclick = () => win?.toggleMaximize();
$('#win-close').onclick = () => win?.close();

// ---------------------------------------------------------------- navegação
function show(name) {
  view = name;
  if (name === 'config') loadAutostart();
  $$('.nav-item').forEach(b => b.classList.toggle('active', b.dataset.view === name));
  $$('.view').forEach(v => v.classList.toggle('active', v.id === `view-${name}`));
  if (name === 'dispositivos') loadPorts();
  if (cfg) renderConfig();
}
$$('.nav-item').forEach(b => (b.onclick = () => show(b.dataset.view)));
$$('[data-goto]').forEach(b => (b.onclick = () => show(b.dataset.goto)));

// ---------------------------------------------------------------- saídas
const viewOf = el => el.closest('.view')?.id.slice(5) ?? view;
const outAt = v => cfg.outputs[pick[v]];
const totalLeds = () => cfg.outputs.reduce((s, o) => s + o.leds.count, 0);
function offsets() {
  let at = 0;
  return cfg.outputs.map(o => ((at += o.leds.count), at - o.leds.count));
}
const currentScale = o => Math.max(1, o.parallel) * Math.max(1, o.leds_per_pixel);
const fullWhite = o => o.leds.count * 3 * MA_PER_CHANNEL * currentScale(o);
const maxDraw = o => (o.power_limit ? Math.min(o.max_current_ma, fullWhite(o)) : fullWhite(o));
const outLabel = o => `${o.name || KIND_NAMES[o.kind]} · D${o.pin}`;

// "@.campo" → "outputs.<i>.campo" da saída escolhida na tela do elemento.
function resolve(path, el) {
  if (!path.startsWith('@.')) return path;
  const i = pick[viewOf(el)];
  return cfg.outputs[i] ? `outputs.${i}.${path.slice(2)}` : null;
}

// ---------------------------------------------------------------- Todos os coolers / fitas
// Campos que "Todos" sincroniza; pino, quantidade, mapa e limites são de cada saída.
const SYNC = ['mode', 'effect', 'audio', 'sound', 'brightness'];
// "Todos" ligado em cada tela (vale quando há mais de uma saída do tipo da aba).
const all = { tela: true, audio: true, efeitos: true };
const ALL_NAMES = { fan: 'Todos os coolers', strip: 'Todas as fitas' };
const PLURAL = { fan: 'coolers', strip: 'fitas' };
const kindIds = kind => cfg.outputs.flatMap((o, i) => (o.kind === kind ? [i] : []));
const groupMode = v => !!tab[v] && all[v] && kindIds(tab[v]).length > 1;
// Referência do grupo: a primeira saída que segue Todos (ou a primeira do tipo).
function refIndex(kind, except = -1) {
  const ids = kindIds(kind).filter(i => i !== except);
  return ids.find(i => cfg.outputs[i].follow !== false) ?? ids[0] ?? -1;
}
function copySync(from, to) {
  for (const k of SYNC) to[k] = structuredClone(from[k]);
}
function groupCfg(kind) {
  cfg.groups ??= {};
  return (cfg.groups[kind] ??= { phase: 0, overwrite: false });
}

// Grava `rel` (ex.: "effect.speed") na saída escolhida ou, em "Todos", no grupo.
// Devolve true se alguma saída passou a seguir Todos ou deixou de seguir.
function setOut(v, rel, value) {
  const sync = SYNC.includes(rel.split('.')[0]);
  const before = cfg.outputs.map(o => o.follow !== false).join();
  if (!groupMode(v)) {
    const o = cfg.outputs[pick[v]];
    if (!o) return false;
    setPath(o, rel, structuredClone(value));
    // Mexer num grupo sozinho dá a ele ajustes próprios.
    if (sync && kindIds(o.kind).length > 1) o.follow = false;
    return before !== cfg.outputs.map(x => x.follow !== false).join();
  }
  const kind = tab[v];
  const ref = cfg.outputs[refIndex(kind)];
  const overwrite = groupCfg(kind).overwrite;
  setPath(ref, rel, structuredClone(value));
  if (sync) ref.follow = true;
  for (const i of kindIds(kind)) {
    const o = cfg.outputs[i];
    if (o === ref || (o.follow === false && !overwrite)) continue;
    if (sync) {
      // Tudo igual ao grupo, não só o campo mexido.
      copySync(ref, o);
      o.follow = true;
    } else {
      setPath(o, rel, structuredClone(value));
    }
  }
  return before !== cfg.outputs.map(x => x.follow !== false).join();
}

function clampPicks() {
  const n = cfg.outputs.length;
  for (const k of Object.keys(pick)) {
    if (tab[k] && groupMode(k)) {
      pick[k] = refIndex(tab[k]);
    } else if (tab[k]) {
      // A saída escolhida tem de ser do tipo da aba; senão, a primeira dele.
      if (cfg.outputs[pick[k]]?.kind !== tab[k]) pick[k] = cfg.outputs.findIndex(o => o.kind === tab[k]);
    } else {
      pick[k] = Math.max(0, Math.min(pick[k], n - 1));
    }
  }
}

const C = (r, g, b) => ({ r, g, b });
const GOLD = C(0xc9, 0xa5, 0x6b), BLUE = C(0x2e, 0x7c, 0xd6);

// Mesmos padrões de OutputCfg::fan / ::strip no Rust. Saída nova sempre com limite.
function newOutput(kind) {
  const used = new Set(cfg.outputs.map(o => o.pin));
  const pins = pinRange();
  const pin = [6, 9, 5, 3, 7, 8, 4, 2, ...pins].find(p => pins.includes(p) && !used.has(p)) ?? pins[0];
  const fan = kind === 'fan';
  return {
    name: fan ? 'Cooler' : 'Fita',
    pin, kind, color_order: 2,
    parallel: 1,
    leds_per_pixel: fan ? 2 : 1,
    mode: fan ? 'effect' : 'screen',
    brightness: 1, power_limit: true, max_current_ma: 6000,
    leds: { count: fan ? 11 : 60, offset: 0, reverse: false, map: [], points: [], shape: kind, fans: 1 },
    zones: { left: 11, top: 19, right: 11, bottom: 19, depth: 0.12, preset: 'full' },
    effect: { kind: 'rainbow', colors: [GOLD, BLUE], speed: 4, size: 0.5, angle: 0, fan_phase: 0 },
    audio: { mode: fan ? 'space' : 'spectrum', sensitivity: 5, colors: [BLUE, GOLD] },
    sound: { enabled: false, amount: 0.6, floor: 0.4, band: 'bass', boost: 1.5, vivid: true },
    follow: true,
    cascade: kindIds(kind).length,
  };
}

// ---------------------------------------------------------------- configuração
const getPath = (o, p) => p.split('.').reduce((x, k) => x?.[k], o);
function setPath(o, p, v) {
  const ks = p.split('.');
  const last = ks.pop();
  ks.reduce((x, k) => x[k], o)[last] = v;
}
const hex = c => '#' + [c.r, c.g, c.b].map(v => v.toString(16).padStart(2, '0')).join('');
const rgb = h => ({ r: parseInt(h.slice(1, 3), 16), g: parseInt(h.slice(3, 5), 16), b: parseInt(h.slice(5, 7), 16) });

function readInput(el) {
  switch (el.dataset.type) {
    case 'int': return parseInt(el.value, 10);
    case 'float': return parseFloat(el.value);
    case 'bool': return el.value === 'true';
    case 'color': return rgb(el.value);
  }
  return el.type === 'checkbox' ? el.checked : el.value;
}

function writeInput(el, v) {
  if (v === undefined || el === document.activeElement) return;
  if (el.type === 'checkbox') el.checked = !!v;
  else if (el.dataset.type === 'color') el.value = hex(v);
  else el.value = String(v);
  paintRange(el);
}

const FORMATS = {
  pct: v => `${Math.round(v * 100)}%`,
  dec1: v => Number(v).toFixed(1),
  kelvin: v => `${v}K`,
  deg: v => `${v}°`,
  phase: v => (v ? `${Math.round(v * 100)}%` : 'Juntas'),
  gphase: v => (v ? `${Math.round(v * 100)}%` : 'Juntos'),
};

function paintRange(el) {
  if (el.type !== 'range') return;
  const p = ((el.value - el.min) / (el.max - el.min)) * 100;
  el.style.setProperty('--p', `${p}%`);
  const out = el.nextElementSibling;
  if (out?.classList.contains('val')) {
    const f = FORMATS[out.dataset.fmt];
    out.textContent = f ? f(Number(el.value)) : el.value;
  }
}

let saveTimer;
let saving = false;
function save() {
  clearTimeout(saveTimer);
  saving = true;
  saveTimer = setTimeout(
    () => invoke('set_config', { config: cfg }).catch(fail).finally(() => (saving = false)),
    120,
  );
  renderDerived();
}

function renderConfig() {
  clampPicks();
  renderTabs();
  renderPickers();
  const ed = outAt('dispositivos');
  if (ed) $('#leds-offset').max = Math.max(0, zoneTotal(ed.zones) - 1);
  $$('[data-bind]').forEach(el => {
    const p = resolve(el.dataset.bind, el);
    if (p) writeInput(el, getPath(cfg, p));
  });
  $$('.mode-grid').forEach(g => {
    const p = resolve(g.dataset.group, g);
    const v = p && getPath(cfg, p);
    $$('.mode-card', g).forEach(c => c.classList.toggle('active', c.dataset.value === v));
  });
  // Campos que só valem para fan ou só para fita.
  $$('[data-kind]').forEach(el => {
    const o = outAt(viewOf(el));
    el.classList.toggle('off', !o || o.kind !== el.dataset.kind);
  });
  const fx = outAt('efeitos');
  if (fx) {
    $$('[data-effects]').forEach(el => el.classList.toggle('off', !el.dataset.effects.split(' ').includes(fx.effect.kind)));
    $('#fan-phase-field').classList.toggle('off', fx.kind !== 'fan' || (fx.leds.fans || 1) < 2);
    $('#size-label').textContent = SIZE_LABELS[fx.effect.kind] || 'Tamanho';
  }
  $('#leds-map').value = ed ? ed.leds.map.join(', ') : '';
  // Som nos modos de tela: "Reagir ao som" (opcional) ou Tela + Som (sempre).
  // Os ajustes aparecem só com o som valendo; "Cor sempre viva" só nas fans.
  const so = outAt('tela');
  const sm = so?.mode;
  const always = sm === 'screen_audio';
  $('#sound-card').hidden = !so || !['screen', 'screen_color', 'screen_audio'].includes(sm);
  $('#sound-title').textContent = always ? 'Tela + Som' : 'Reagir ao som';
  $('#sound-toggle').hidden = always;
  $('#sound-amount').hidden = always;
  $('#sound-vivid').hidden = so?.kind !== 'fan';
  $('#sound-card .sound-fields').hidden = !(always || so?.sound?.enabled);
  renderZones();
  renderCascade();
  renderOutputs();
  renderEditor();
  renderMapStatus();
  renderDerived();
}

// Campos que mudam a lista de saídas ou o que o Arduino precisa saber.
const STRUCTURAL = /^outputs\.\d+\.(name|pin|kind|mode|color_order|parallel|leds_per_pixel|power_limit|max_current_ma|leds\.count|sound\.enabled)$/;

document.addEventListener('input', e => {
  const el = e.target;
  if (!el.dataset.bind || !cfg) return;
  const path = resolve(el.dataset.bind, el);
  if (!path) return;
  const v = readInput(el);
  if (typeof v === 'number' && Number.isNaN(v)) return;
  // Um grupo que ganhou ou perdeu ajustes próprios muda os avisos: redesenha.
  const regrouped = el.dataset.bind.startsWith('@.') && setOut(viewOf(el), el.dataset.bind.slice(2), v);
  if (!el.dataset.bind.startsWith('@.')) setPath(cfg, path, v);
  if (path.endsWith('.kind')) changeKind(getPath(cfg, path.slice(0, -5)), v);
  $$('[data-bind]').forEach(o => o !== el && resolve(o.dataset.bind, o) === path && writeInput(o, v));
  paintRange(el);
  if (regrouped || STRUCTURAL.test(path) || path.endsWith('.kind')) renderConfig();
  save();
});

function changeKind(o, kind) {
  o.leds.shape = kind;
  if (!KIND_MODES[kind].includes(o.mode)) o.mode = KIND_MODES[kind][0];
  o.leds_per_pixel = kind === 'fan' ? 2 : 1;
  if (kind === 'strip') o.parallel = 1;
}

// ---------------------------------------------------------------- abas Cooler / Fita
function renderTabs() {
  $$('.kind-tabs').forEach(bar => {
    const v = viewOf(bar);
    $$('.kind-tab', bar).forEach(b => {
      const n = cfg.outputs.filter(o => o.kind === b.dataset.tab).length;
      b.classList.toggle('active', tab[v] === b.dataset.tab);
      $('.n', b).textContent = n > 1 ? n : '';
    });
  });
}
$$('.kind-tab').forEach(b =>
  b.addEventListener('click', () => {
    const v = viewOf(b);
    tab[v] = b.dataset.tab;
    all[v] = true;
    pick[v] = cfg.outputs.findIndex(o => o.kind === tab[v]);
    renderConfig();
  }),
);

// ---------------------------------------------------------------- escolha da saída nas telas
function renderPickers() {
  $$('.out-card').forEach(card => {
    const v = viewOf(card);
    const box = $('.out-pick', card);
    box.innerHTML = '';
    // Só as saídas do tipo da aba; com uma só, não precisa escolher.
    const mine = cfg.outputs.map((o, i) => [o, i]).filter(([o]) => !tab[v] || o.kind === tab[v]);
    box.hidden = mine.length < 2;
    const grouped = groupMode(v);
    if (mine.length >= 2) {
      const b = document.createElement('button');
      b.className = `out-chip all ${grouped ? 'active' : ''}`;
      b.innerHTML = `<i class="${tab[v]}"></i><span></span>`;
      $('span', b).textContent = `${ALL_NAMES[tab[v]]} (${mine.length})`;
      b.onclick = () => {
        all[v] = true;
        renderConfig();
      };
      box.append(b);
    }
    mine.forEach(([o, i]) => {
      const b = document.createElement('button');
      b.className = `out-chip ${!grouped && i === pick[v] ? 'active' : ''}`;
      b.innerHTML = `<i class="${o.kind}"></i><span></span>${o.follow === false ? '<em class="own">próprio</em>' : ''}`;
      $('span', b).textContent = `${outLabel(o)} · ${MODE_NAMES[o.mode]}`;
      b.onclick = () => {
        all[v] = false;
        pick[v] = i;
        renderConfig();
      };
      box.append(b);
    });
    renderGroupNote(card, v);
    const o = outAt(v);
    $('h2', card).lastChild.textContent = grouped
      ? `${ALL_NAMES[tab[v]]} (${mine.length})`
      : o ? outLabel(o) : TAB_NAMES[tab[v]] || 'Saída';
    $('.out-hint', card).textContent = o
      ? MODE_HINTS[o.mode]
      : `Nenhuma saída ${TAB_NAMES[tab[v]] || ''}. Adicione uma em Dispositivos.`;
  });
  // Opções de modo conforme o tipo da saída de cada tela.
  $$('.mode-sel').forEach(sel => {
    const o = outAt(viewOf(sel));
    sel.innerHTML = '';
    if (!o) return;
    KIND_MODES[o.kind].forEach(m => sel.append(new Option(MODE_NAMES[m], m)));
  });
  const pin = $('#ed-pin');
  const ed = outAt('dispositivos');
  pin.innerHTML = '';
  const pins = pinRange();
  pins.forEach(p => {
    const other = cfg.outputs.some((o, i) => o.pin === p && i !== pick.dispositivos);
    pin.append(new Option(`D${p}${other ? ' (em uso)' : ''}`, p));
  });
  if (ed && !pins.includes(ed.pin)) pin.append(new Option(`D${ed.pin} (o firmware não aceita)`, ed.pin));
  if (ed) pin.value = ed.pin;
}

// Aviso embaixo da escolha: o que "Todos" vai mudar, ou que o grupo tem ajustes próprios.
function renderGroupNote(card, v) {
  const note = $('.group-note', card);
  note.innerHTML = '';
  const kind = tab[v];
  const ids = kind ? kindIds(kind) : [];
  if (ids.length < 2) return;
  if (groupMode(v)) {
    const own = ids.filter(i => cfg.outputs[i].follow === false);
    const p = document.createElement('span');
    p.textContent = `Vale para os ${ids.length} ${PLURAL[kind]} ao mesmo tempo.` +
      (own.length === 1 ? ' 1 grupo com ajustes próprios fica como está.' : own.length ? ` ${own.length} grupos com ajustes próprios ficam como estão.` : '');
    note.append(p);
    if (own.length) {
      const t = document.createElement('label');
      t.className = 'toggle';
      t.innerHTML = '<input type="checkbox"><span></span>Aplicar também nos grupos com ajustes próprios';
      const box = $('input', t);
      box.checked = groupCfg(kind).overwrite;
      box.onchange = () => {
        groupCfg(kind).overwrite = box.checked;
        save();
      };
      note.append(t);
    }
    return;
  }
  const o = cfg.outputs[pick[v]];
  if (!o) return;
  if (o.follow === false) {
    const box = document.createElement('div');
    box.className = 'own-box';
    box.innerHTML = '<span>Este grupo tem ajustes próprios.</span><button class="btn small">Voltar a seguir Todos</button>';
    $('button', box).onclick = () => {
      const r = refIndex(o.kind, pick[v]);
      if (r >= 0 && cfg.outputs[r].follow !== false) copySync(cfg.outputs[r], o);
      o.follow = true;
      all[v] = true;
      renderConfig();
      save();
    };
    note.append(box);
  } else {
    const p = document.createElement('span');
    p.textContent = 'Segue Todos. Mudar algo aqui dá ajustes próprios a este grupo.';
    note.append(p);
  }
}

// Cascata entre grupos (tela Efeitos, com "Todos").
function renderCascade() {
  const card = $('#cascade-card');
  const on = groupMode('efeitos');
  card.hidden = !on;
  if (!on) return;
  const kind = tab.efeitos;
  const range = $('#group-phase');
  range.value = groupCfg(kind).phase || 0;
  paintRange(range);
  const ids = kindIds(kind).sort((a, b) => (cfg.outputs[a].cascade || 0) - (cfg.outputs[b].cascade || 0) || a - b);
  const list = $('#cascade-list');
  list.innerHTML = '';
  ids.forEach((i, rank) => {
    const o = cfg.outputs[i];
    const row = document.createElement('div');
    row.className = `cascade-row ${o.follow === false ? 'out' : ''}`;
    row.innerHTML = `<span class="n">${rank + 1}</span><span class="who"></span>
      <button class="icon-btn" title="Antes">↑</button><button class="icon-btn" title="Depois">↓</button>`;
    $('.who', row).textContent = outLabel(o) + (o.follow === false ? ' (ajustes próprios, fora da cascata)' : '');
    const [up, down] = $$('button', row);
    up.disabled = rank === 0;
    down.disabled = rank === ids.length - 1;
    const move = d => {
      [ids[rank], ids[rank + d]] = [ids[rank + d], ids[rank]];
      ids.forEach((j, r) => (cfg.outputs[j].cascade = r));
      renderCascade();
      save();
    };
    up.onclick = () => move(-1);
    down.onclick = () => move(1);
    list.append(row);
  });
}
$('#group-phase').addEventListener('input', e => {
  groupCfg(tab.efeitos).phase = parseFloat(e.target.value);
  paintRange(e.target);
  save();
});

// ---------------------------------------------------------------- lista de saídas (Dispositivos)
const KIND_ICON = { fan: '#i-gear', strip: '#i-grid' };

function renderOutputs() {
  const list = $('#out-list');
  list.innerHTML = '';
  const pins = {};
  const allowed = pinRange();
  cfg.outputs.forEach(o => (pins[o.pin] = (pins[o.pin] || 0) + 1));
  cfg.outputs.forEach((o, i) => {
    const row = document.createElement('div');
    row.className = `out-row ${i === pick.dispositivos ? 'sel' : ''} ${pins[o.pin] > 1 || !allowed.includes(o.pin) ? 'dup' : ''}`;
    row.innerHTML = `
      <span class="kind-ico ${o.kind}"><svg><use href="${KIND_ICON[o.kind]}"/></svg></span>
      <div class="out-main"><b></b><small></small></div>
      <span class="out-ma"></span>
      <div class="out-btns">
        <button class="icon-btn" data-act="up" title="Subir">↑</button>
        <button class="icon-btn" data-act="down" title="Descer">↓</button>
        <button class="icon-btn" data-act="test" title="Testar esta saída"><svg><use href="#i-play"/></svg></button>
        <button class="icon-btn danger" data-act="del" title="Remover">✕</button>
      </div>`;
    $('b', row).textContent = o.name || KIND_NAMES[o.kind];
    const par = o.kind === 'fan' && o.parallel > 1 ? ` × ${o.parallel} em paralelo` : '';
    const follow = kindIds(o.kind).length > 1 ? (o.follow === false ? ' · ajustes próprios' : ' · segue Todos') : '';
    $('small', row).textContent =
      `D${o.pin}${pins[o.pin] > 1 ? ' (pino repetido!)' : allowed.includes(o.pin) ? '' : ' (pino fora da faixa!)'} · ${KIND_NAMES[o.kind]} · ${o.leds.count} px${par} · ${ORDER_NAMES[o.color_order]} · ${MODE_NAMES[o.mode]}${follow}`;
    $('[data-act=up]', row).disabled = i === 0;
    $('[data-act=down]', row).disabled = i === cfg.outputs.length - 1;
    row.onclick = e => {
      const act = e.target.closest('[data-act]')?.dataset.act;
      if (act) return outputAction(act, i, e.target.closest('button'));
      pick.dispositivos = i;
      renderConfig();
    };
    list.append(row);
  });
  renderOutputCurrents();
  const total = totalLeds();
  $('#outs-total').textContent = `${total} de ${maxLeds()} LEDs • ${cfg.outputs.length} de ${MAX_OUTPUTS} saídas`;
  const warns = [];
  Object.entries(pins).forEach(([p, n]) => n > 1 && warns.push(`O pino <b>D${p}</b> está em ${n} saídas: cada saída precisa de um pino só dela.`));
  if (total > maxLeds()) warns.push(`São <b>${total}</b> LEDs; o Arduino aceita até ${maxLeds()} somando todas as saídas.`);
  const outside = cfg.outputs.filter(o => !allowed.includes(o.pin)).map(o => `D${o.pin}`);
  if (outside.length) warns.push(`O firmware só aceita os pinos D${allowed[0]} a D${allowed[allowed.length - 1]}: troque ${outside.join(', ')}.`);
  if (cfg.outputs.length > MAX_OUTPUTS) warns.push(`O firmware aceita até ${MAX_OUTPUTS} saídas.`);
  if (!cfg.outputs.length) warns.push('Nenhuma saída. Adicione uma fan ou uma fita.');
  $('#outs-warn').hidden = !warns.length;
  $('#outs-warn').innerHTML = warns.join('<br>');
  $('#add-fan').disabled = $('#add-strip').disabled = cfg.outputs.length >= MAX_OUTPUTS;
}

// Corrente de cada saída: medida com o Halo rodando, máxima estimada parado.
function renderOutputCurrents() {
  const live = status.running && lastFrame?.currents?.length === cfg.outputs.length;
  $$('#out-list .out-ma').forEach((el, i) => {
    const o = cfg.outputs[i];
    if (!o) return;
    el.textContent = live ? fmtA(lastFrame.currents[i]) : `até ${fmtA(maxDraw(o))}`;
    el.classList.toggle('hot', !o.power_limit && fullWhite(o) > 2000);
    el.title = o.power_limit ? `Limite: ${fmtA(o.max_current_ma)}` : 'Sem limite de consumo';
  });
}

async function outputAction(act, i, btn) {
  const outs = cfg.outputs;
  if (act === 'up' || act === 'down') {
    const j = act === 'up' ? i - 1 : i + 1;
    [outs[i], outs[j]] = [outs[j], outs[i]];
    pick.dispositivos = j;
  } else if (act === 'del') {
    const o = outs[i];
    if (!confirm(`Remover a saída "${o.name || KIND_NAMES[o.kind]}" (D${o.pin})?`)) return;
    outs.splice(i, 1);
  } else if (act === 'test') {
    return testOutput(i, btn);
  }
  renderConfig();
  save();
}

function addOutput(kind) {
  cfg.outputs.push(newOutput(kind));
  pick.dispositivos = cfg.outputs.length - 1;
  renderConfig();
  save();
}
$('#add-fan').onclick = () => addOutput('fan');
$('#add-strip').onclick = () => addOutput('strip');

async function testOutput(i, btn) {
  const o = cfg.outputs[i];
  if (!cfg.serial.port) return toast('Escolha a porta COM do Arduino primeiro.', 'err');
  if (btn) btn.disabled = true;
  toast(`Testando ${outLabel(o)}: vermelho, verde, azul e um ponto correndo.`);
  try {
    // Garante que o backend tem a lista atual antes de abrir a porta.
    clearTimeout(saveTimer);
    await invoke('set_config', { config: cfg });
    const fw = await invoke('test_leds', { output: i });
    toast(`Teste de ${outLabel(o)} concluído. ${fw.reply || ''}`, fw.ok === false ? 'err' : 'ok');
  } catch (e) {
    fail(e);
  }
  if (btn) btn.disabled = false;
  pollStatus();
}

// ---------------------------------------------------------------- editor da saída
function renderEditor() {
  const o = outAt('dispositivos');
  $('#out-editor').hidden = !o;
  if (!o) return;
  $('#ed-title').textContent = `Editando: ${outLabel(o)}`;
  const full = fullWhite(o);
  const par = o.kind === 'fan' ? ` (${o.leds.count} px × ${currentScale(o)} LEDs físicos por pixel)` : '';
  $('#ed-power').textContent = o.power_limit
    ? `Branco total puxaria ${fmtA(full)}${par}; o limite segura em ${fmtA(Math.min(full, o.max_current_ma))}.`
    : `Branco total: ${fmtA(full)}${par}.`;
  const warn = $('#ed-nolimit');
  warn.hidden = o.power_limit;
  warn.innerHTML = `<b>Limite de consumo desligado.</b> Esta saída pode puxar até <b>${fmtA(full)}</b> em branco.
    Fio fino de 5 V esquenta com isso. Deixe o limite ligado, ou use fio grosso e uma fonte própria para os LEDs.`;
}

$('#btn-test-out').onclick = e => testOutput(pick.dispositivos, e.currentTarget);
$('#btn-zones').onclick = () => {
  pick.tela = pick.dispositivos;
  show('tela');
};

// ---------------------------------------------------------------- zonas (fita escolhida na Tela)
const EDGES = ['left', 'top', 'right', 'bottom'];
const zoneTotal = z => EDGES.reduce((s, k) => s + z[k], 0);
const telaStrip = () => {
  const o = outAt('tela');
  return o && o.kind === 'strip' ? o : null;
};

function monitorAspect() {
  const m = monitors[cfg?.capture.monitor ?? 0];
  return m ? m.width / m.height : 16 / 9;
}

function distribute(total, preset) {
  const a = monitorAspect();
  const w = { left: 1, top: a, right: 1, bottom: a };
  if (preset === 'top_sides') w.bottom = 0;
  if (preset === 'top') w.left = w.right = w.bottom = 0;
  if (preset === 'sides') w.top = w.bottom = 0;
  const sum = EDGES.reduce((s, k) => s + w[k], 0);
  const out = {};
  EDGES.forEach(k => (out[k] = Math.round((total * w[k]) / sum)));
  const diff = total - EDGES.reduce((s, k) => s + out[k], 0);
  out[w.top ? 'top' : 'left'] += diff;
  return out;
}

function setZones(edges, preset) {
  const o = telaStrip();
  if (!o) return;
  const before = zoneTotal(o.zones);
  Object.assign(o.zones, edges);
  o.zones.preset = preset;
  // Fita com 1 LED por zona: acompanha o total de zonas.
  if (o.leds.count === before && !o.leds.map.length) o.leds.count = Math.max(1, Math.min(maxLeds(), zoneTotal(o.zones)));
  o.leds.offset = Math.min(o.leds.offset, Math.max(0, zoneTotal(o.zones) - 1));
  renderConfig();
  save();
}

function renderZones() {
  const o = telaStrip();
  if (!o) return;
  const total = zoneTotal(o.zones);
  const range = $('#zones-total');
  range.value = total;
  paintRange(range);
  $('#zones-total-out').textContent = total;
  $('#zones-preset').value = o.zones.preset || 'custom';
  $$('[data-edge]').forEach(i => (i.value = o.zones[i.dataset.edge]));
}

$('#zones-total').addEventListener('input', e => {
  const o = telaStrip();
  if (!o) return;
  const preset = o.zones.preset === 'custom' ? 'full' : o.zones.preset;
  setZones(distribute(parseInt(e.target.value, 10), preset), preset);
});
$('#zones-preset').addEventListener('change', e => {
  const o = telaStrip();
  if (o && e.target.value !== 'custom') setZones(distribute(zoneTotal(o.zones), e.target.value), e.target.value);
});
$$('[data-edge]').forEach(i =>
  i.addEventListener('change', () => {
    const v = Math.max(0, Math.min(maxLeds(), parseInt(i.value, 10) || 0));
    setZones({ [i.dataset.edge]: v }, 'custom');
  }),
);

// ---------------------------------------------------------------- modos e cores
// Escolher um efeito (ou uma visualização de música) põe a saída nesse modo.
$$('.mode-grid').forEach(g =>
  $$('.mode-card', g).forEach(c =>
    c.addEventListener('click', () => {
      const p = resolve(g.dataset.group, g);
      if (!p) return;
      const v = viewOf(g);
      setOut(v, g.dataset.group.slice(2), c.dataset.value);
      if (g.dataset.mode && getPath(cfg, resolve('@.mode', g)) !== 'screen_audio') setOut(v, 'mode', g.dataset.mode);
      renderConfig();
      save();
    }),
  ),
);
$$('.swatches').forEach(s =>
  $$('button', s).forEach(b =>
    b.addEventListener('click', () => {
      if (!resolve(s.dataset.target, s)) return;
      const css = getComputedStyle(b);
      setOut(viewOf(s), s.dataset.target.slice(2), [rgb(css.getPropertyValue('--a').trim()), rgb(css.getPropertyValue('--b').trim())]);
      renderConfig();
      save();
    }),
  ),
);

// ---------------------------------------------------------------- iniciar / parar
$$('.start-btn').forEach(b =>
  b.addEventListener('click', async () => {
    if (status.running) return;
    $('span', b).textContent = cfg.serial.simulate ? 'Iniciando…' : 'Conectando ao Arduino…';
    $$('.start-btn').forEach(x => (x.disabled = true));
    try {
      clearTimeout(saveTimer);
      await invoke('set_config', { config: cfg });
      await invoke('start');
      lastError = null;
    } catch (e) {
      fail(e);
    }
    $$('.start-btn').forEach(x => (x.disabled = false));
    pollStatus();
  }),
);
$$('.stop-btn').forEach(b => b.addEventListener('click', () => invoke('stop').then(pollStatus)));

function runningSummary() {
  return cfg.outputs.filter(o => o.mode !== 'off').map(o => `${o.name || KIND_NAMES[o.kind]}: ${MODE_NAMES[o.mode]}`).join(' • ');
}

async function pollStatus() {
  try {
    status = await invoke('status');
  } catch {
    return;
  }
  const running = status.running;
  $$('.start-btn').forEach(b => {
    b.classList.toggle('running', running);
    if (!b.disabled) $('span', b).textContent = running ? 'Halo ativo' : 'Iniciar Halo';
  });
  $$('.stop-btn').forEach(b => (b.disabled = !running));
  const np = $('#now-playing');
  $('.dot', np).className = `dot ${running ? 'ok' : ''}`;
  $('.txt', np).textContent = running ? 'Rodando' : 'Parado';
  np.title = running && cfg ? runningSummary() : '';
  $('#cfg-path').textContent = status.config_path || '—';
  // O backend acha o Arduino em outra COM quando ele troca de USB: acompanha.
  if (cfg && !saving && status.port && status.port !== cfg.serial.port) {
    cfg.serial.port = status.port;
    loadPorts();
  }
  if (status.error && status.error !== lastError) fail(status.error);
  lastError = status.error;
  renderOutputState();
  renderOutputCurrents();
}

function renderOutputState() {
  if (!cfg) return;
  const running = !!status.running;
  const sim = cfg.serial.simulate;
  const port = cfg.serial.port || 'sem porta';
  let dot = '', title, sub;
  if (status.error && !running) [dot, title, sub] = ['err', 'Erro', status.error];
  else if (sim) [dot, title, sub] = ['warn', 'Simulação', `Quadros Adalight só no log • ${port}`];
  else if (running) [dot, title, sub] = ['ok', 'Conectado', `Arduino UNO • ${port}`];
  else [dot, title, sub] = ['', 'Pronto', `Arduino UNO • ${port} (abre ao iniciar)`];
  const chip = $('#chip-output');
  $('.dot', chip).className = `dot ${dot}`;
  $('.txt', chip).textContent = sim ? 'Simulação' : running ? `Conectado ${port}` : port;
  $('#info-out').textContent = title;
  $('#info-out-sub').textContent = `${sim ? 'Arduino UNO (simulado)' : `Arduino UNO • ${port}`}${
    cfg.openrgb.enabled && cfg.openrgb.devices.length ? ` + OpenRGB (${cfg.openrgb.devices.length})` : ''
  }`;
  $('#dev-dot').className = `big-dot ${dot}`;
  $('#dev-state').textContent = title;
  $('#dev-sub').textContent = sub;
  renderFirmware();
}

// O que o Arduino disse sobre as saídas (resposta ao Hlo? / HloC).
function renderFirmware() {
  const fw = status.firmware || {};
  const line = $('#fw-line');
  const want = cfg.outputs.map(o => `${o.pin}:${o.leds.count}:${o.color_order}`).join(',');
  const pretty = s => s.split(',').filter(Boolean).map(x => {
    const [p, n, ord] = x.split(':');
    return `D${p}: ${n} ${ORDER_NAMES[ord] ?? ''}`;
  }).join(', ');
  let cls = '', text;
  if (!fw.version) text = 'Firmware: ainda não conectado. A configuração das saídas vai ao iniciar ou em "Enviar saídas".';
  else if (fw.version === 'simulado') [cls, text] = ['warn', `Simulação: as saídas (${pretty(fw.outputs)}) não foram enviadas.`];
  else if (fw.version === 'v1') [cls, text] = [cfg.outputs.length > 1 ? 'err' : 'warn', `${fw.reply}. Para usar vários pinos, grave o firmware v2.`];
  else if (!fw.ok) [cls, text] = ['err', `Firmware v2: ${fw.reply}. No Arduino: ${pretty(fw.outputs) || '—'}.`];
  else if (fw.outputs !== want) [cls, text] = ['warn', `Firmware v2: o Arduino está com ${pretty(fw.outputs)}. Clique em "Enviar saídas" para aplicar a lista atual.`];
  else [cls, text] = ['ok', `Firmware v2 • ${fw.reply} • ${pretty(fw.outputs)}`];
  line.className = `hint ${cls}`;
  line.textContent = text;
}

$('#btn-apply-outs').addEventListener('click', async e => {
  const b = e.currentTarget;
  b.disabled = true;
  try {
    clearTimeout(saveTimer);
    await invoke('set_config', { config: cfg });
    const fw = await invoke('apply_outputs');
    toast(fw.reply || 'Saídas enviadas.', fw.ok ? 'ok' : 'err');
  } catch (err) {
    fail(err);
  }
  b.disabled = false;
  pollStatus();
});

function renderDerived() {
  const total = totalLeds();
  const strip = telaStrip();
  $('#info-leds').textContent = `${total} LEDs`;
  $('#info-zones').textContent = strip
    ? `${zoneTotal(strip.zones)} zonas em ${strip.name || 'Fita'}`
    : `${cfg.outputs.length} saída${cfg.outputs.length === 1 ? '' : 's'}`;
  $('#chip-leds').textContent = total;
  const bytes = 6 + 3 * total;
  const frameMs = (bytes * 10 * 1000) / cfg.serial.baud + (total * SHOW_US_PER_LED) / 1000;
  const maxFps = Math.floor(1000 / frameMs);
  const hint = $('#baud-hint');
  hint.textContent = `Com ${total} LEDs a ${cfg.serial.baud} baud cabem cerca de ${maxFps} quadros/s (envio + o Arduino acendendo os LEDs).`;
  hint.style.color = maxFps < cfg.capture.fps ? 'var(--warn)' : '';
  renderOutputState();
}

// ---------------------------------------------------------------- monitores e portas
async function loadMonitors() {
  try {
    monitors = await invoke('list_monitors');
  } catch (e) {
    fail(e);
  }
  const sel = $('#sel-monitor');
  sel.innerHTML = '';
  monitors.forEach(m => {
    const o = new Option(`${m.index + 1} · ${m.primary ? 'Principal' : 'Secundário'} · ${m.width}×${m.height}`, m.index);
    sel.append(o);
  });
  if (cfg) sel.value = cfg.capture.monitor;
}

async function loadPorts() {
  let ports = [];
  try {
    ports = await invoke('list_ports');
  } catch (e) {
    fail(e);
  }
  const sel = $('#sel-port');
  sel.innerHTML = '';
  sel.append(new Option(ports.length ? 'Escolha a porta' : 'Nenhuma porta encontrada', ''));
  ports.forEach(p => sel.append(new Option(p.label + (p.arduino ? ' (Arduino?)' : ''), p.name)));
  if (cfg.serial.port && !ports.some(p => p.name === cfg.serial.port)) {
    sel.append(new Option(`${cfg.serial.port} (desconectada)`, cfg.serial.port));
  }
  // Só pré-seleciona; a porta continua fechada até testar, mapear ou iniciar.
  if (!cfg.serial.port) {
    const guess = ports.find(p => p.arduino);
    if (guess) {
      cfg.serial.port = guess.name;
      save();
    }
  }
  sel.value = cfg.serial.port;
}
$('#btn-ports').onclick = loadPorts;

$('#btn-map').addEventListener('click', () => {
  const o = outAt('dispositivos');
  if (!o) return;
  const text = $('#leds-map').value.trim();
  const map = text ? text.split(/[\s,;]+/).filter(Boolean).map(Number) : [];
  if (map.some(n => !Number.isInteger(n))) return toast('Use só números inteiros separados por vírgula.', 'err');
  o.leds.map = map;
  if (map.length && map.length !== o.leds.count) {
    toast(`A lista tem ${map.length} números, mas a saída tem ${o.leds.count} LEDs: ajustei a quantidade.`);
    o.leds.count = Math.min(maxLeds(), map.length);
  }
  renderConfig();
  save();
});

// ---------------------------------------------------------------- OpenRGB
$('#btn-openrgb').addEventListener('click', async () => {
  const list = $('#openrgb-list');
  list.innerHTML = '<p class="hint">Procurando…</p>';
  let devices;
  try {
    devices = await invoke('openrgb_devices', { host: cfg.openrgb.host, port: cfg.openrgb.port });
  } catch (e) {
    list.innerHTML = '';
    const p = document.createElement('p');
    p.className = 'hint';
    p.textContent = String(e);
    return list.append(p);
  }
  list.innerHTML = '';
  if (!devices.length) list.innerHTML = '<p class="hint">O OpenRGB não encontrou dispositivos.</p>';
  devices.forEach(d => {
    const item = document.createElement('label');
    item.className = `device-item ${d.streamable ? '' : 'locked'}`;
    const box = document.createElement('input');
    box.type = 'checkbox';
    box.disabled = !d.streamable;
    box.checked = d.streamable && cfg.openrgb.devices.includes(d.index);
    box.onchange = () => {
      const set = new Set(cfg.openrgb.devices);
      box.checked ? set.add(d.index) : set.delete(d.index);
      cfg.openrgb.devices = [...set];
      save();
    };
    const text = document.createElement('div');
    text.innerHTML = '<b></b><small></small>';
    $('b', text).textContent = d.name;
    $('small', text).textContent = `${d.vendor || 'OpenRGB'} • ${d.leds} LEDs`;
    const badge = document.createElement('span');
    badge.className = `badge ${d.streamable ? 'ok' : 'err'}`;
    badge.textContent = d.streamable ? 'Direct: pode animar' : 'Grava na memória: bloqueado';
    item.append(box, text, badge);
    list.append(item);
  });
});

// ---------------------------------------------------------------- iniciar com o Windows
function renderAutostart(a) {
  const box = $('#opt-autostart');
  box.checked = a.enabled;
  box.disabled = a.dev && !a.enabled;
  $('#autostart-info').textContent = a.dev
    ? 'Esta é a versão de desenvolvimento (npm run dev). Instale o Halo Studio para ele iniciar com o Windows.'
    : a.enabled
      ? `Liga sozinho no login, só com o ícone na bandeja.${a.current ? '' : ' O registro aponta para outro exe: desligue e ligue de novo.'}`
      : 'Desligado: o Halo só abre quando você abrir.';
}
async function loadAutostart() {
  try {
    renderAutostart(await invoke('get_autostart'));
  } catch (e) {
    fail(e);
  }
}
$('#opt-autostart').addEventListener('change', async e => {
  try {
    const a = await invoke('set_autostart', { enabled: e.target.checked });
    renderAutostart(a);
    toast(a.enabled ? 'O Halo Studio vai iniciar com o Windows.' : 'Início automático desligado.', 'ok');
  } catch (err) {
    fail(err);
    loadAutostart();
  }
});
$('#btn-quit').onclick = () => invoke('quit_app');

// ---------------------------------------------------------------- configurações gerais
const AUTOSTART_KEY = 'halo.autostart';
function storage(k, v) {
  try {
    if (v === undefined) return localStorage.getItem(k) || '';
    localStorage.setItem(k, v);
  } catch {
    return '';
  }
}
$('#sel-autostart').value = storage(AUTOSTART_KEY) ? 'on' : '';
$('#sel-autostart').onchange = e => storage(AUTOSTART_KEY, e.target.value);
$('#btn-reset').addEventListener('click', async () => {
  if (!confirm('Voltar todas as configurações para o padrão? A lista de saídas volta a ser uma fan no D6.')) return;
  // O backend preenche o que falta com o padrão.
  await invoke('set_config', { config: {} }).catch(fail);
  cfg = await invoke('get_config');
  renderConfig();
  loadPorts();
  toast('Configurações restauradas.', 'ok');
});
$('#opt-halo').addEventListener('change', e => $('#stage-tela').classList.toggle('no-halo', !e.target.checked));

// ---------------------------------------------------------------- quadros
function parseFrame(buf) {
  const dv = new DataView(buf);
  let o = 0;
  const u16 = () => ((o += 2), dv.getUint16(o - 2, true));
  const f32 = () => ((o += 4), dv.getFloat32(o - 4, true));
  const f = { seq: dv.getUint32(0, true) };
  o = 4;
  f.w = u16(); f.h = u16();
  const zones = u16(), leds = u16(), bands = u16();
  const flags = u16();
  f.running = !!(flags & 1);
  f.fps = f32(); f.captureFps = f32(); f.latency = f32(); f.ma = f32();
  f.content = [f32(), f32(), f32(), f32()];
  f.rects = Array.from({ length: zones }, () => [f32(), f32(), f32(), f32()]);
  if (f.w) {
    const len = f.w * f.h * 4;
    f.img = new Uint8ClampedArray(buf.slice(o, o + len));
    o += len;
  }
  f.zones = new Uint8Array(buf, o, zones * 3); o += zones * 3;
  f.leds = new Uint8Array(buf, o, leds * 3); o += leds * 3;
  f.bands = Array.from({ length: bands }, f32);
  f.currents = Array.from({ length: flags >> 8 }, f32);
  return f;
}

function renderStats(f) {
  const r = f.running;
  $$('.st-fps').forEach(e => (e.textContent = r ? Math.round(f.fps) : 0));
  $$('.st-lat').forEach(e => (e.textContent = r ? `${f.latency.toFixed(1)} ms` : '— ms'));
  // Barra: total contra o limite global, ou contra a soma dos limites das saídas.
  const limit = cfg.color.power_limit ? cfg.color.max_current_ma : cfg.outputs.reduce((s, o) => s + maxDraw(o), 0) || 1;
  const ma = r ? f.ma : 0;
  const pct = Math.min(100, (ma / limit) * 100);
  $$('.st-ma').forEach(e => (e.textContent = cfg.color.power_limit ? `${fmtA(ma)} (${Math.round(pct)}%)` : fmtA(ma)));
  $$('.st-meter').forEach(e => (e.style.width = `${pct}%`));
  $('#chip-fps').textContent = r ? Math.round(f.fps) : 0;
  const screen = cfg.outputs.some(o => ['screen', 'screen_color', 'screen_audio'].includes(o.mode));
  $('#info-capture').textContent = r && screen ? `Tempo real • ${Math.round(f.fps)} FPS` : r ? 'Sem saída na tela' : 'Parada';
  renderOutputCurrents();
}

// Ajusta os canvas ao tamanho do palco. O do halo fica em 1/4 da resolução:
// ele é borrado pelo CSS de qualquer jeito.
function fit(stage) {
  const r = stage.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const scene = $('canvas.scene', stage);
  const halo = $('canvas.halo', stage);
  const sw = Math.round(r.width * dpr), sh = Math.round(r.height * dpr);
  if (scene.width !== sw || scene.height !== sh) {
    scene.width = sw;
    scene.height = sh;
    halo.width = Math.max(1, Math.round(r.width / 4));
    halo.height = Math.max(1, Math.round(r.height / 4));
  }
  const ctx = scene.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const hctx = halo.getContext('2d');
  hctx.setTransform(0.25, 0, 0, 0.25, 0, 0);
  return { w: r.width, h: r.height, ctx, hctx };
}

function monitorRect(w, h, aspect, margin) {
  let mw = w - 2 * margin;
  let mh = mw / aspect;
  if (mh > h - 2 * margin) {
    mh = h - 2 * margin;
    mw = mh * aspect;
  }
  return { x: (w - mw) / 2, y: (h - mh) / 2, w: mw, h: mh };
}

function roundRect(ctx, x, y, w, h, r) {
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, Math.max(0, r));
}

// Borda de cada zona, na ordem do backend: esquerda, topo, direita, base.
function zoneEdges(z, count) {
  if (!z || zoneTotal(z) !== count) return null;
  return [...Array(z.left).fill('l'), ...Array(z.top).fill('t'), ...Array(z.right).fill('r'), ...Array(z.bottom).fill('b')];
}

const rgbCss = (a, i, k = 1) => `rgb(${a[i * 3] * k},${a[i * 3 + 1] * k},${a[i * 3 + 2] * k})`;
const offscreen = document.createElement('canvas');

function drawTela(f) {
  const stage = $('#stage-tela');
  const { w, h, ctx, hctx } = fit(stage);
  $('#tela-empty').classList.toggle('hidden', !!f.img);
  ctx.clearRect(0, 0, w, h);
  hctx.clearRect(0, 0, w, h);
  const aspect = f.w ? f.w / f.h : monitorAspect();
  const tile = Math.max(8, Math.min(18, Math.min(w, h) * 0.035));
  const gap = 6;
  const m = monitorRect(w, h, aspect, tile + gap + 16);
  const c = {
    x: m.x + f.content[0] * m.w, y: m.y + f.content[1] * m.h,
    w: (f.content[2] - f.content[0]) * m.w, h: (f.content[3] - f.content[1]) * m.h,
  };

  if (f.img) {
    if (offscreen.width !== f.w || offscreen.height !== f.h) {
      offscreen.width = f.w;
      offscreen.height = f.h;
    }
    offscreen.getContext('2d').putImageData(new ImageData(f.img, f.w, f.h), 0, 0);
    ctx.save();
    roundRect(ctx, m.x, m.y, m.w, m.h, 6);
    ctx.clip();
    ctx.imageSmoothingQuality = 'high';
    ctx.drawImage(offscreen, m.x, m.y, m.w, m.h);
    // Escurece o que fica fora da área útil (barra de tarefas, faixas pretas).
    ctx.fillStyle = 'rgba(5,10,16,.6)';
    ctx.fillRect(m.x, m.y, m.w, c.y - m.y);
    ctx.fillRect(m.x, c.y + c.h, m.w, m.y + m.h - c.y - c.h);
    ctx.fillRect(m.x, c.y, c.x - m.x, c.h);
    ctx.fillRect(c.x + c.w, c.y, m.x + m.w - c.x - c.w, c.h);
    ctx.restore();
  } else {
    const g = ctx.createLinearGradient(0, m.y, 0, m.y + m.h);
    g.addColorStop(0, '#0E1A27');
    g.addColorStop(1, '#09121B');
    ctx.fillStyle = g;
    roundRect(ctx, m.x, m.y, m.w, m.h, 6);
    ctx.fill();
  }
  ctx.strokeStyle = '#2A3E52';
  ctx.lineWidth = 1.5;
  roundRect(ctx, m.x - 1, m.y - 1, m.w + 2, m.h + 2, 7);
  ctx.stroke();

  const strip = telaStrip();
  if (!strip) {
    // Aba Cooler: a borda toda na cor que vai para a fan.
    const o = outAt('tela');
    if (!o || !f.running) return;
    const off = offsets()[pick.tela];
    if (f.leds.length < (off + 1) * 3) return;
    hctx.fillStyle = rgbCss(f.leds, off);
    hctx.fillRect(m.x - 50, m.y - 50, m.w + 100, m.h + 100);
    return;
  }
  if ($('#opt-areas').checked) {
    ctx.strokeStyle = 'rgba(201,165,107,.55)';
    ctx.lineWidth = 1;
    f.rects.forEach(r => ctx.strokeRect(c.x + r[0] * c.w, c.y + r[1] * c.h, (r[2] - r[0]) * c.w, (r[3] - r[1]) * c.h));
  }
  const edges = zoneEdges(strip.zones, f.rects.length);
  if (!edges) return;
  const lit = f.running && strip.mode === 'screen' && f.zones.length;
  f.rects.forEach((r, i) => {
    let x, y, tw, th, hx, hy, hw, hh;
    const reach = 60;
    switch (edges[i]) {
      case 'l': x = m.x - gap - tile; y = c.y + r[1] * c.h; tw = tile; th = (r[3] - r[1]) * c.h;
        [hx, hy, hw, hh] = [x - reach, y, reach + tile, th]; break;
      case 'r': x = m.x + m.w + gap; y = c.y + r[1] * c.h; tw = tile; th = (r[3] - r[1]) * c.h;
        [hx, hy, hw, hh] = [x, y, reach + tile, th]; break;
      case 't': x = c.x + r[0] * c.w; y = m.y - gap - tile; tw = (r[2] - r[0]) * c.w; th = tile;
        [hx, hy, hw, hh] = [x, y - reach, tw, reach + tile]; break;
      default: x = c.x + r[0] * c.w; y = m.y + m.h + gap; tw = (r[2] - r[0]) * c.w; th = tile;
        [hx, hy, hw, hh] = [x, y, tw, reach + tile];
    }
    const pad = Math.min(1.5, Math.min(tw, th) * 0.15);
    ctx.fillStyle = lit ? rgbCss(f.zones, i) : '#1A2633';
    roundRect(ctx, x + pad, y + pad, tw - 2 * pad, th - 2 * pad, 3);
    ctx.fill();
    if (lit) {
      hctx.fillStyle = ctx.fillStyle;
      hctx.fillRect(hx, hy, hw, hh);
    }
  });
}

// Zona de cada LED da fita (mesma conta de zones::led_map no Rust).
function ledMap(l, zones) {
  const n = l.count;
  if (l.map.length === n) return l.map.map(z => (z >= 0 && z < zones ? z : -1));
  if (!zones) return Array(n).fill(-1);
  return Array.from({ length: n }, (_, i) => {
    const pos = l.reverse ? n - 1 - i : i;
    const z = Math.floor((pos * zones) / n);
    return (((z + l.offset) % zones) + zones) % zones;
  });
}

// Fita em volta do monitor: cada LED na borda da zona que ele mostra.
function drawRing(stage, o, leds, lit, rects, opts = {}) {
  const { w, h, ctx, hctx } = fit(stage);
  ctx.clearRect(0, 0, w, h);
  hctx.clearRect(0, 0, w, h);
  const led = Math.max(6, Math.min(12, Math.min(w, h) * 0.03));
  const gap = 8;
  const m = monitorRect(w, h, monitorAspect(), led + gap + 18);
  const g = ctx.createLinearGradient(0, m.y, 0, m.y + m.h);
  g.addColorStop(0, '#0F1C2A');
  g.addColorStop(1, '#09111A');
  ctx.fillStyle = g;
  roundRect(ctx, m.x, m.y, m.w, m.h, 8);
  ctx.fill();
  ctx.strokeStyle = '#2A3E52';
  ctx.lineWidth = 1.5;
  ctx.stroke();

  if (opts.bands?.length && lit) {
    // Espectro dentro do monitor, graves à esquerda.
    const [c0, c1] = o.audio.colors;
    const n = opts.bands.length;
    const bw = (m.w - 40) / n;
    opts.bands.forEach((v, i) => {
      const k = i / (n - 1);
      const mix = (a, b) => Math.round(a + (b - a) * k);
      ctx.fillStyle = `rgb(${mix(c0.r, c1.r)},${mix(c0.g, c1.g)},${mix(c0.b, c1.b)})`;
      const bh = Math.max(2, v * (m.h - 40));
      roundRect(ctx, m.x + 20 + i * bw + 2, m.y + m.h - 20 - bh, bw - 4, bh, 2);
      ctx.fill();
    });
  }

  const edges = zoneEdges(o.zones, rects.length);
  if (!edges) return;
  const map = ledMap(o.leds, rects.length);
  const pos = map.map(z => {
    if (z < 0) return null;
    const r = rects[z];
    const cx = m.x + ((r[0] + r[2]) / 2) * m.w, cy = m.y + ((r[1] + r[3]) / 2) * m.h;
    switch (edges[z]) {
      case 'l': return [m.x - gap - led / 2, cy, -1, 0];
      case 'r': return [m.x + m.w + gap + led / 2, cy, 1, 0];
      case 't': return [cx, m.y - gap - led / 2, 0, -1];
      default: return [cx, m.y + m.h + gap + led / 2, 0, 1];
    }
  });
  pos.forEach((p, i) => {
    if (!p) return;
    ctx.fillStyle = lit ? rgbCss(leds, i) : '#1C2A38';
    roundRect(ctx, p[0] - led / 2, p[1] - led / 2, led, led, 3);
    ctx.fill();
    if (lit) {
      hctx.fillStyle = ctx.fillStyle;
      hctx.beginPath();
      hctx.arc(p[0] + p[2] * 20, p[1] + p[3] * 20, 34, 0, Math.PI * 2);
      hctx.fill();
    }
  });

  if (opts.markStart) {
    // Marca o LED 1 em dourado e a direção para o LED 2.
    const a = pos.find(Boolean);
    const b = a && pos.slice(pos.indexOf(a) + 1).find(p => p && (p[0] !== a[0] || p[1] !== a[1]));
    if (a) {
      ctx.strokeStyle = '#C9A56B';
      ctx.lineWidth = 2;
      roundRect(ctx, a[0] - led / 2 - 3, a[1] - led / 2 - 3, led + 6, led + 6, 4);
      ctx.stroke();
      ctx.fillStyle = '#C9A56B';
      ctx.font = '600 11px Inter, sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText('LED 1', a[0] - a[2] * 46, a[1] - a[3] * 40 + 4);
      if (b) {
        const ang = Math.atan2(b[1] - a[1], b[0] - a[0]);
        const sx = a[0] - a[2] * 18, sy = a[1] - a[3] * 18;
        ctx.save();
        ctx.translate(sx, sy);
        ctx.rotate(ang);
        ctx.beginPath();
        ctx.moveTo(-10, 0);
        ctx.lineTo(10, 0);
        ctx.moveTo(5, -4);
        ctx.lineTo(10, 0);
        ctx.lineTo(5, 4);
        ctx.stroke();
        ctx.restore();
      }
    }
  }
}

// Desenha a saída escolhida numa tela: fan pelo mapa, fita em volta do monitor.
function drawOutput(stage, i, f, opts = {}) {
  const o = cfg.outputs[i];
  if (!o) {
    const { w, h, ctx, hctx } = fit(stage);
    ctx.clearRect(0, 0, w, h);
    hctx.clearRect(0, 0, w, h);
    return;
  }
  let leds = opts.leds;
  let lit = !!leds;
  if (!leds) {
    const off = offsets()[i];
    lit = f.running && f.leds.length >= (off + o.leds.count) * 3;
    leds = lit ? f.leds.subarray(off * 3, (off + o.leds.count) * 3) : new Uint8Array(0);
  }
  if (o.kind === 'fan') drawMapped(stage, o, leds, lit);
  else drawRing(stage, o, leds, lit, f.rects, { ...opts, bands: opts.bands && f.bands });
}

let inflight = false;
let lastReq = 0;
function frameLoop(t) {
  requestAnimationFrame(frameLoop);
  const preview = view === 'efeitos' && !status.running && pick.efeitos >= 0 && cfg?.outputs.length;
  const every = status.running || preview ? 30 : 250;
  if (inflight || !cfg || t - lastReq < every) return;
  lastReq = t;
  inflight = true;
  const i = pick[view] ?? -1;
  const output = view === 'tela' ? (telaStrip() ? i : null) : i >= 0 ? i : null;
  invoke('get_frame', { withImage: view === 'tela', output })
    .then(async buf => {
      const f = parseFrame(buf);
      lastFrame = f;
      renderStats(f);
      if (view === 'tela') drawTela(f);
      else if (view === 'audio') drawOutput($('#stage-audio'), i, f, { bands: true });
      else if (view === 'efeitos') {
        // Halo parado: o backend calcula o efeito só para mostrar aqui.
        const leds = preview ? new Uint8Array(await invoke('effect_preview', { output: i })) : undefined;
        drawOutput($('#stage-efeitos'), i, f, { leds });
      } else if (view === 'dispositivos') drawOutput($('#stage-leds'), i, f, { markStart: true });
    })
    .catch(() => {})
    .finally(() => (inflight = false));
}

// ---------------------------------------------------------------- mapa de pixels (fans)
const isMapped = l => l.points.some(p => p.length);

// Mesma conta de visuals::positions no Rust: (ângulo a partir do topo no
// sentido horário, distância do centro), ou a ordem na fita sem mapa.
function ledPositions(l) {
  const n = Math.max(1, l.count);
  const fans = Math.max(1, l.fans || 1);
  return Array.from({ length: n }, (_, i) => {
    const pt = l.points[i]?.[0];
    if (!pt) {
      const p = (i + 0.5) / n;
      return [p, Math.abs(2 * p - 1)];
    }
    let dx = pt[0] - 0.5;
    if (l.shape === 'fan') {
      const cell = Math.min(fans - 1, Math.max(0, Math.floor(pt[0] * fans)));
      dx = pt[0] * fans - cell - 0.5;
    }
    const dy = pt[1] - 0.5;
    return [(Math.atan2(dy, dx) / (2 * Math.PI) + 1.25) % 1, Math.min(1, Math.hypot(dx, dy) / 0.5)];
  });
}

function hsv(h) {
  const f = n => {
    const k = (n + h * 6) % 6;
    return Math.round(255 * (1 - Math.max(0, Math.min(1, Math.min(k, 4 - k)))));
  };
  return { r: f(5), g: f(3), b: f(1) };
}

// Área do desenho (fans lado a lado, quadradas; ou um retângulo 16:9).
function shapeArea(w, h, shape, fans, pad = 18) {
  const aspect = shape === 'fan' ? Math.max(1, fans) : 16 / 9;
  let aw = w - 2 * pad;
  let ah = aw / aspect;
  if (ah > h - 2 * pad) {
    ah = h - 2 * pad;
    aw = ah * aspect;
  }
  return { x: (w - aw) / 2, y: (h - ah) / 2, w: aw, h: ah };
}
const toPx = (a, pt) => [a.x + pt[0] * a.w, a.y + pt[1] * a.h];

function drawShape(ctx, a, shape, fans) {
  ctx.lineWidth = 1.5;
  if (shape === 'fan') {
    const cw = a.w / Math.max(1, fans);
    for (let i = 0; i < fans; i++) {
      const x = a.x + i * cw;
      const cx = x + cw / 2, cy = a.y + a.h / 2;
      ctx.fillStyle = '#0E1823';
      ctx.strokeStyle = '#2A3E52';
      roundRect(ctx, x + 5, a.y + 5, cw - 10, a.h - 10, cw * 0.08);
      ctx.fill();
      ctx.stroke();
      ctx.strokeStyle = '#33506A';
      for (const r of [0.41, 0.2]) {
        ctx.beginPath();
        ctx.arc(cx, cy, cw * r, 0, Math.PI * 2);
        ctx.stroke();
      }
      ctx.fillStyle = '#1C2A38';
      for (const [sx, sy] of [[0.1, 0.1], [0.9, 0.1], [0.1, 0.9], [0.9, 0.9]]) {
        ctx.beginPath();
        ctx.arc(x + sx * cw, a.y + sy * a.h, cw * 0.025, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  } else {
    ctx.fillStyle = '#0E1823';
    ctx.strokeStyle = '#2A3E52';
    roundRect(ctx, a.x, a.y, a.w, a.h, 10);
    ctx.fill();
    ctx.stroke();
    ctx.setLineDash([5, 5]);
    ctx.strokeStyle = '#26394B';
    roundRect(ctx, a.x + a.w * 0.12, a.y + a.h * 0.14, a.w * 0.76, a.h * 0.72, 6);
    ctx.stroke();
    ctx.setLineDash([]);
  }
}

// Pontos de cada pixel no desenho: os do mapa ou, sem mapa, um círculo pela ordem.
function fanPoints(l) {
  const fans = Math.max(1, l.fans || 1);
  return Array.from({ length: l.count }, (_, i) => {
    if (l.points[i]?.length) return l.points[i];
    const per = Math.ceil(l.count / fans);
    const cell = Math.min(fans - 1, Math.floor(i / per));
    const a = ((i % per) / per) * Math.PI * 2;
    return [[(cell + 0.5 + 0.41 * Math.sin(a)) / fans, 0.5 - 0.41 * Math.cos(a)]];
  });
}

// Fan: o desenho com cada pixel aceso na cor que está indo para ele.
function drawMapped(stage, o, leds, lit) {
  const { w, h, ctx, hctx } = fit(stage);
  ctx.clearRect(0, 0, w, h);
  hctx.clearRect(0, 0, w, h);
  const l = o.leds;
  const a = shapeArea(w, h, 'fan', l.fans);
  drawShape(ctx, a, 'fan', l.fans);
  const r = Math.max(4, Math.min(a.w / Math.max(1, l.fans), a.h) * 0.035);
  fanPoints(l).forEach((pts, i) =>
    pts.forEach(pt => {
      const [x, y] = toPx(a, pt);
      ctx.fillStyle = lit && leds.length >= (i + 1) * 3 ? rgbCss(leds, i) : '#22344A';
      ctx.beginPath();
      ctx.arc(x, y, r, 0, Math.PI * 2);
      ctx.fill();
      if (lit) {
        hctx.fillStyle = ctx.fillStyle;
        hctx.beginPath();
        hctx.arc(x, y, r * 5, 0, Math.PI * 2);
        hctx.fill();
      }
    }),
  );
}

function renderMapStatus() {
  const o = outAt('dispositivos');
  if (!o || o.kind !== 'fan') return;
  const l = o.leds;
  const n = l.points.filter(p => p.length).length;
  $('#map-status').textContent = n
    ? `Mapa: ${n} de ${l.count} pixels. Os efeitos usam as posições.`
    : 'Sem mapa: os efeitos seguem a ordem dos pixels.';
  $('#btn-unmap').disabled = !n;
}

$('#btn-unmap').addEventListener('click', () => {
  const o = outAt('dispositivos');
  if (!o) return;
  o.leds.points = [];
  save();
  renderMapStatus();
});

// ---------------------------------------------------------------- assistente
const M = { step: 'setup', out: 0, shape: 'fan', fans: 1, count: 11, points: [], cur: 0, keep: 0, raf: 0 };
const OFF = { r: 0, g: 0, b: 0 };

function mapStep(step) {
  M.step = step;
  $$('#mapper .map-step').forEach(s => s.classList.toggle('active', s.dataset.step === step));
  cancelAnimationFrame(M.raf);
  if (step === 'map') {
    light();
    drawMapper();
  }
  if (step === 'preview') previewLoop();
}

function openMapper() {
  const o = outAt('dispositivos');
  M.out = pick.dispositivos;
  M.shape = 'fan';
  M.fans = o.leds.fans || 1;
  M.count = o.leds.count;
  $('#map-shape').value = 'fan';
  $('#map-shape').closest('.field').hidden = true;
  $('#map-fans').value = M.fans;
  $('#map-count').value = M.count;
  $('#mapper').hidden = false;
  mapStep('setup');
}

async function closeMapper() {
  clearInterval(M.keep);
  cancelAnimationFrame(M.raf);
  $('#mapper').hidden = true;
  await invoke('mapper_close').catch(() => {});
}

function send(colors) {
  return invoke('mapper_send', { colors }).catch(e => {
    fail(e);
    closeMapper();
  });
}

// Acende só o pixel atual, em vermelho moderado.
function light() {
  return send(Array.from({ length: M.count }, (_, i) => (i === M.cur ? { r: 190, g: 0, b: 0 } : OFF)));
}

$('#btn-mapper').addEventListener('click', () => {
  if (!cfg.serial.port) return toast('Escolha a porta COM do Arduino primeiro.', 'err');
  if (outAt('dispositivos')?.kind !== 'fan') return;
  openMapper();
});
$('#map-x').onclick = closeMapper;
$$('[data-map-close]').forEach(b => (b.onclick = closeMapper));
$('#map-fans').oninput = e => {
  M.fans = Math.max(1, Math.min(12, parseInt(e.target.value, 10) || 1));
  $('#map-count').value = M.fans * 11;
};

$('#map-begin').addEventListener('click', async e => {
  const b = e.currentTarget;
  const label = b.lastChild;
  const o = cfg.outputs[M.out];
  M.count = Math.max(1, Math.min(maxLeds(), parseInt($('#map-count').value, 10) || 11));
  M.fans = Math.max(1, Math.min(12, parseInt($('#map-fans').value, 10) || 1));
  // A quantidade de pixels faz parte da configuração das saídas no Arduino.
  if (o.leds.count !== M.count) {
    o.leds.count = M.count;
    renderConfig();
  }
  b.disabled = true;
  label.textContent = `Abrindo ${cfg.serial.port}…`;
  try {
    clearTimeout(saveTimer);
    await invoke('set_config', { config: cfg });
    const fw = await invoke('mapper_open', { output: M.out });
    if (fw.version === 'v2' && !fw.ok) toast(`Arduino: ${fw.reply}`, 'err');
  } catch (err) {
    fail(err);
    return;
  } finally {
    b.disabled = false;
    label.textContent = 'Começar';
  }
  const l = o.leds;
  const same = l.points.length === M.count && (l.fans || 1) === M.fans;
  M.points = same ? l.points.map(p => p.map(q => [...q])) : Array.from({ length: M.count }, () => []);
  M.cur = 0;
  // O firmware apaga tudo depois de 5 s sem quadro: reenviamos a cada 1 s.
  clearInterval(M.keep);
  M.keep = setInterval(() => M.step === 'map' && light(), 1000);
  mapStep('map');
});

function mapCanvas(id) {
  const c = $(id);
  const r = c.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  if (c.width !== Math.round(r.width * dpr) || c.height !== Math.round(r.height * dpr)) {
    c.width = Math.round(r.width * dpr);
    c.height = Math.round(r.height * dpr);
  }
  const ctx = c.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, r.width, r.height);
  return { ctx, a: shapeArea(r.width, r.height, M.shape, M.fans, 24), r };
}

function marker(ctx, x, y, label, color, filled) {
  ctx.beginPath();
  ctx.arc(x, y, 10, 0, Math.PI * 2);
  ctx.lineWidth = 2;
  ctx.strokeStyle = color;
  ctx.fillStyle = filled ? color : 'rgba(10,17,25,.85)';
  ctx.fill();
  ctx.stroke();
  ctx.fillStyle = filled ? '#0A1119' : color;
  ctx.font = '600 10px Inter, sans-serif';
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(label, x, y + 0.5);
}

function drawMapper() {
  const { ctx, a } = mapCanvas('#map-canvas');
  drawShape(ctx, a, M.shape, M.fans);
  M.points.forEach((pts, i) =>
    pts.forEach((pt, k) => {
      const [x, y] = toPx(a, pt);
      marker(ctx, x, y, String(i + 1), i === M.cur ? '#E06868' : '#C9A56B', k === 0);
    }),
  );
  $('#map-title').textContent = `Pixel ${M.cur + 1} de ${M.count}`;
  $('#map-back').disabled = M.cur === 0;
}

function next() {
  if (M.cur >= M.count - 1) return mapStep('preview');
  M.cur++;
  light();
  drawMapper();
}

function pointAt(e) {
  const { a, r } = mapCanvas('#map-canvas');
  const x = (e.clientX - r.left - a.x) / a.w;
  const y = (e.clientY - r.top - a.y) / a.h;
  return [x, y].map(v => Math.round(Math.min(1, Math.max(0, v)) * 1000) / 1000);
}

function secondPoint(pt) {
  const pts = M.points[M.cur];
  M.points[M.cur] = pts.length ? [pts[0], pt] : [pt];
  drawMapper();
}

$('#map-canvas').addEventListener('click', e => {
  const pt = pointAt(e);
  if (e.shiftKey) return secondPoint(pt);
  M.points[M.cur] = [pt, ...M.points[M.cur].slice(1)];
  drawMapper();
  next();
});
$('#map-canvas').addEventListener('contextmenu', e => {
  e.preventDefault();
  secondPoint(pointAt(e));
});

function back() {
  if (M.cur === 0) return;
  M.cur--;
  light();
  drawMapper();
}
function skip() {
  M.points[M.cur] = [];
  next();
}
function redo() {
  M.points = Array.from({ length: M.count }, () => []);
  M.cur = 0;
  mapStep('map');
}
$('#map-back').onclick = back;
$('#map-skip').onclick = skip;
$('#map-redo').onclick = redo;
$('#map-redo2').onclick = redo;
$('#map-done').onclick = () => mapStep('preview');
$('#map-adjust').onclick = () => mapStep('map');

document.addEventListener('keydown', e => {
  if ($('#mapper').hidden || e.target.matches('input, select, textarea')) return;
  if (e.key === 'Escape') return closeMapper();
  if (M.step !== 'map') return;
  const k = e.key.toLowerCase();
  if (k === 's') skip();
  else if (k === 'r') redo();
  else if (k === 'backspace' || k === 'arrowleft') back();
  else if (k === 'enter') mapStep('preview');
  else return;
  e.preventDefault();
});

// Onda de arco-íris girando pelas posições marcadas, no desenho e na fan.
function previewLoop() {
  const pos = ledPositions({ count: M.count, points: M.points, shape: M.shape, fans: M.fans });
  let lastSend = 0;
  let busy = false;
  const tick = t => {
    M.raf = requestAnimationFrame(tick);
    const colors = pos.map(([p]) => hsv((p - t / 2500 + 100) % 1));
    if (!busy && t - lastSend > 33) {
      lastSend = t;
      busy = true;
      invoke('mapper_send', { colors: colors.map(c => ({ r: c.r >> 1, g: c.g >> 1, b: c.b >> 1 })) })
        .catch(e => {
          fail(e);
          closeMapper();
        })
        .finally(() => (busy = false));
    }
    const { ctx, a } = mapCanvas('#map-preview');
    drawShape(ctx, a, M.shape, M.fans);
    M.points.forEach((pts, i) =>
      pts.forEach(pt => {
        const [x, y] = toPx(a, pt);
        const c = colors[i];
        const g = ctx.createRadialGradient(x, y, 0, x, y, 28);
        g.addColorStop(0, `rgba(${c.r},${c.g},${c.b},.9)`);
        g.addColorStop(1, `rgba(${c.r},${c.g},${c.b},0)`);
        ctx.fillStyle = g;
        ctx.beginPath();
        ctx.arc(x, y, 28, 0, Math.PI * 2);
        ctx.fill();
        ctx.fillStyle = `rgb(${c.r},${c.g},${c.b})`;
        ctx.beginPath();
        ctx.arc(x, y, 6, 0, Math.PI * 2);
        ctx.fill();
      }),
    );
  };
  M.raf = requestAnimationFrame(tick);
}

$('#map-save').addEventListener('click', async () => {
  const o = cfg.outputs[M.out];
  Object.assign(o.leds, { points: M.points, shape: 'fan', fans: M.fans, count: M.count });
  if (o.leds.map.length && o.leds.map.length !== M.count) o.leds.map = [];
  renderConfig();
  save();
  await closeMapper();
  toast(`Mapa salvo: ${M.points.filter(p => p.length).length} de ${M.count} pixels.`, 'ok');
});

// ---------------------------------------------------------------- início
(async function init() {
  try {
    cfg = await invoke('get_config');
  } catch (e) {
    return fail(e);
  }
  // Tela começa na Fita (zonas); Áudio e Efeitos no Cooler. Sem saída do
  // tipo, a aba do outro tipo.
  const has = k => cfg.outputs.some(o => o.kind === k);
  tab.tela = has('strip') || !has('fan') ? 'strip' : 'fan';
  tab.audio = tab.efeitos = has('fan') || !has('strip') ? 'fan' : 'strip';
  await loadMonitors();
  renderConfig();
  await pollStatus();
  setInterval(pollStatus, 500);
  requestAnimationFrame(frameLoop);
  loadPorts();
  if (storage(AUTOSTART_KEY) && !status.running) $('.start-btn')?.click();
})();
