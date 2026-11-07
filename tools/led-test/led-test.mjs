// Teste e mapeamento dos LEDs ligados no Arduino (firmware halo-adalight).
//
//   node led-test.mjs info             mostra as saídas configuradas no Arduino (firmware v2)
//   node led-test.mjs config 6:11:GRB 9:60:GRB   configura as saídas: pino:quantidade:ordem
//   node led-test.mjs andar            um LED por vez; Enter avança, "v" volta, "q" sai (--cor FF0000)
//   node led-test.mjs bytes            um canal de cor (byte) por vez: revela RGB vs RGBW
//   node led-test.mjs contar           numera por cor (vermelho, amarelo, grupos de 5)
//   node led-test.mjs grupos           faixas de cor: 1-6, 7-10, 11-14, 15-18, 19+
//   node led-test.mjs cor FF0000       tudo numa cor (--de 1 --ate 6 = só essa faixa)
//   node led-test.mjs arcoiris         animação a 60 FPS e mede o FPS real
//
// Opções: --porta COMx (padrão: acha o Arduino sozinho) --baud 1000000 --leds 30 --brilho 55 --segundos 30 --fps 60

import { SerialPort } from "serialport";
import readline from "node:readline";

const args = process.argv.slice(2);
const opt = (name, def) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? args[i + 1] : def;
};
const modo = args[0] && !args[0].startsWith("--") ? args[0] : "andar";
// Sem --porta, procura o Arduino pelo chip USB (CH340 = 1a86; Arduino original = 2341).
const porta = opt("porta", null) ?? (await SerialPort.list())
  .find((p) => ["1a86", "2341"].includes(p.vendorId?.toLowerCase()))?.path;
if (!porta) {
  console.error("Arduino não encontrado. Ligue no USB ou use --porta COMx.");
  process.exit(1);
}
console.log(`Arduino em ${porta}`);
const baud = Number(opt("baud", 1000000));
const leds = Number(opt("leds", 30));
const brilho = Math.max(0, Math.min(100, Number(opt("brilho", 55)))) / 100;
const segundos = Number(opt("segundos", 30));
const fps = Number(opt("fps", 60));

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function frame(colorOf) {
  const n = leds - 1;
  const hi = (n >> 8) & 0xff, lo = n & 0xff;
  const buf = Buffer.alloc(6 + leds * 3);
  buf.set([0x41, 0x64, 0x61, hi, lo, hi ^ lo ^ 0x55]);
  for (let i = 0; i < leds; i++) {
    const [r, g, b] = colorOf(i) ?? [0, 0, 0];
    buf[6 + i * 3] = r * brilho;
    buf[7 + i * 3] = g * brilho;
    buf[8 + i * 3] = b * brilho;
  }
  return buf;
}

function hsv(h) {
  const f = (n) => {
    const k = (n + h * 6) % 6;
    return Math.round(255 * (1 - Math.max(0, Math.min(1, Math.min(k, 4 - k)))));
  };
  return [f(5), f(3), f(1)];
}

const port = new SerialPort({ path: porta, baudRate: baud, autoOpen: false });
await new Promise((ok, fail) => port.open((e) => (e ? fail(e) : ok())));
const send = (buf) => new Promise((ok) => port.write(buf, () => port.drain(ok)));

// Texto que o Arduino manda ("Ada", respostas "Hlo ..."), para os comandos de saídas.
let recebido = "";
port.on("data", (d) => { recebido += d.toString("latin1"); });
async function esperarLinha(prefixo, ms = 1500) {
  const fim = Date.now() + ms;
  while (Date.now() < fim) {
    const linha = recebido.split("\n").find((l) => l.startsWith(prefixo));
    if (linha) return linha.trim();
    await sleep(20);
  }
  return null;
}

// Abrir a porta reinicia o Arduino: espera o bootloader e o teste de boot.
await sleep(2200);

// Reenvia o último quadro a cada 1 s para o firmware não apagar por inatividade.
let atual = frame(() => null);
const manter = setInterval(() => send(atual), 1000);
const mostrar = async (buf) => { atual = buf; await send(buf); };

async function sair() {
  clearInterval(manter);
  await send(frame(() => null));
  await new Promise((ok) => port.close(ok));
  process.exit(0);
}
process.on("SIGINT", sair);

const ORDENS = ["RGB", "RBG", "GRB", "GBR", "BRG", "BGR"];

if (modo === "info") {
  recebido = "";
  await send(Buffer.from("Hlo?", "latin1"));
  console.log((await esperarLinha("Hlo")) ?? "Sem resposta: o Arduino está com o firmware antigo (v1)?");
} else if (modo === "config") {
  // Cada saída como pino:quantidade[:ordem], na ordem do quadro. Ex.: 6:11:GRB 9:60:GRB
  const saidas = args.slice(1).filter((a) => /^\d+:\d+(:[A-Za-z]{3})?$/.test(a)).map((a) => {
    const [pino, qtd, ordem = "GRB"] = a.split(":");
    const o = ORDENS.indexOf(ordem.toUpperCase());
    if (o < 0) throw new Error(`Ordem de cor inválida: ${ordem} (use ${ORDENS.join(", ")})`);
    return [Number(pino), Number(qtd), o];
  });
  if (saidas.length === 0) throw new Error("Informe as saídas, ex.: config 6:11:GRB 9:60:GRB");
  const corpo = [saidas.length];
  for (const [pino, qtd, o] of saidas) corpo.push(pino, qtd >> 8, qtd & 0xff, o);
  const chk = corpo.reduce((a, b) => a ^ b, 0) ^ 0x55;
  recebido = "";
  await send(Buffer.from([0x48, 0x6c, 0x6f, 0x43, ...corpo, chk]));
  console.log((await esperarLinha("Hlo")) ?? "Sem resposta: o Arduino está com o firmware antigo (v1)?");
  recebido = "";
  await send(Buffer.from("Hlo?", "latin1"));
  console.log((await esperarLinha("Hlo v")) ?? "");
} else if (modo === "descobrir") {
  // Descobre em qual pino está uma fita: cada pino livre acende 30 LEDs de uma cor.
  const cores = { 2: ["VERMELHO", [255, 0, 0]], 3: ["VERDE", [0, 255, 0]], 4: ["AZUL", [0, 0, 255]],
    5: ["AMARELO", [255, 180, 0]], 7: ["ROXO", [180, 0, 255]], 8: ["CIANO", [0, 255, 255]], 9: ["BRANCO", [255, 255, 255]] };
  const pinos = Object.keys(cores).map(Number);
  const corpo = [pinos.length + 1, 6, 0, 11, 2];  // mantém as fans no D6
  for (const p of pinos) corpo.push(p, 0, 30, 2);
  recebido = "";
  await send(Buffer.from([0x48, 0x6c, 0x6f, 0x43, ...corpo, corpo.reduce((a, b) => a ^ b, 0) ^ 0x55]));
  console.log((await esperarLinha("Hlo")) ?? "Sem resposta do firmware v2.");
  for (const p of pinos) console.log(`D${p} = ${cores[p][0]}`);
  console.log(`\nVeja a cor dos primeiros LEDs da fita. Segurando ${segundos} s...`);
  await mostrar(Buffer.concat([Buffer.from([0x41, 0x64, 0x61, 0, 220, 0 ^ 220 ^ 0x55]),
    Buffer.from([...Array(11).fill([0, 0, 0]), ...pinos.flatMap((p) => Array(30).fill(cores[p][1]))]
      .flat().map((v) => v * brilho))]));
  await sleep(segundos * 1000);
  // Volta a configuração só com as fans.
  await send(Buffer.from([0x48, 0x6c, 0x6f, 0x43, 1, 6, 0, 11, 2, (1 ^ 6 ^ 0 ^ 11 ^ 2) ^ 0x55]));
} else if (modo === "fita") {
  // Confere ordem de cor e comprimento: no começo 20 vermelhos, 20 verdes e 20 azuis;
  // no fim, os últimos 10 LEDs em branco. --de/--ate delimitam a fita (1 a N).
  const de = Number(opt("de", 1)) - 1, ate = Number(opt("ate", leds)) - 1;
  console.log(`Início: 20 VERMELHO, 20 VERDE, 20 AZUL. Fim: últimos 10 em BRANCO (LEDs ${ate - 8} a ${ate + 1}).`);
  await mostrar(frame((i) => {
    if (i < de || i > ate) return null;
    const k = i - de;
    if (k < 20) return [255, 0, 0];
    if (k < 40) return [0, 255, 0];
    if (k < 60) return [0, 0, 255];
    return i > ate - 10 ? [255, 255, 255] : null;
  }));
  await sleep(segundos * 1000);
} else if (modo === "diagnostico") {
  // Manda quadros por alguns segundos e conta quantas vezes o Arduino reiniciou
  // (cada boot manda "Ada"). Reinício com a fita acesa = falta de energia.
  const de = Number(opt("de", 1)) - 1, ate = Number(opt("ate", leds)) - 1;
  const quadro = frame((i) => (i >= de && i <= ate && (i - de) % 2 === 0 ? [255, 0, 0] : null));
  recebido = "";
  const inicio = Date.now();
  let enviados = 0;
  while (Date.now() - inicio < segundos * 1000) {
    await send(quadro);
    enviados++;
    await sleep(200);
  }
  await sleep(300);
  const boots = (recebido.match(/Ada\n/g) ?? []).length;
  const acks = (recebido.match(/\x06/g) ?? []).length;
  console.log(`${enviados} quadros enviados em ${segundos} s; ${acks} confirmados pelo Arduino (ACK); "Ada" ${boots} vez(es).`);
  console.log(acks >= enviados ? "Todos os quadros chegaram inteiros." : `${enviados - acks} quadro(s) perdido(s).`);
} else if (modo === "rampa") {
  // Teste de carga: branco na faixa --de/--ate, subindo o brilho em degraus.
  // Ctrl+C apaga na hora. Corrente estimada = 60 mA por LED em branco total.
  const de = Number(opt("de", 1)) - 1, ate = Number(opt("ate", leds)) - 1;
  const degraus = String(opt("degraus", "10,20,30,40")).split(",").map(Number);
  const n = ate - de + 1;
  for (const pct of degraus) {
    const v = Math.round(255 * pct / 100);
    // O brilho do degrau já vai no valor da cor; --brilho não se aplica aqui.
    const buf = frame((i) => (i >= de && i <= ate ? [v / brilho, v / brilho, v / brilho] : null));
    await mostrar(buf);
    console.log(`Branco ${pct}%  ≈ ${(n * 0.06 * pct / 100).toFixed(1)} A estimados (teto do firmware ≈ 6 A). ` +
      `Confira o fio e a cor no FIM da fita. Ctrl+C para apagar.`);
    await sleep(segundos * 1000);
  }
} else if (modo === "andar") {
  console.log(`Acendendo 1 LED por vez (de ${leds}). Enter = próximo, v + Enter = voltar, q + Enter = sair.`);
  console.log("Quando passar do último LED da fan, nada acende: esse é o total.\n");
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  const hex = opt("cor", "FFFFFF");
  const cor = [0, 2, 4].map((o) => parseInt(hex.slice(o, o + 2), 16));
  let i = 0;
  for (;;) {
    await mostrar(frame((k) => (k === i ? cor : null)));
    const resp = (await new Promise((r) => rl.question(`LED ${i + 1} aceso > `, r))).trim().toLowerCase();
    if (resp === "q") break;
    if (resp === "v") i = Math.max(0, i - 1);
    else if (i < leds - 1) i++;
    else { console.log("Fim da lista."); break; }
  }
  rl.close();
} else if (modo === "bytes") {
  // Liga UM byte por vez (um canal de cor de um pixel). Serve para descobrir
  // se a fan usa 3 bytes por LED (RGB) ou outro formato, como 4 (RGBW).
  const canais = ["R", "G", "B"];
  console.log(`Acendendo 1 byte por vez (de ${leds * 3}). Enter = próximo, v + Enter = voltar, q + Enter = sair.\n`);
  const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
  let b = 0;
  for (;;) {
    const buf = frame(() => null);
    buf[6 + b] = 255 * brilho;
    await mostrar(buf);
    const px = Math.floor(b / 3) + 1, ch = canais[b % 3];
    const resp = (await new Promise((r) => rl.question(`Byte ${b + 1} (pixel ${px}, canal ${ch}) > `, r))).trim().toLowerCase();
    if (resp === "q") break;
    if (resp === "v") b = Math.max(0, b - 1);
    else if (b < leds * 3 - 1) b++;
    else break;
  }
  rl.close();
} else if (modo === "contar") {
  console.log("LED 1 = VERMELHO, LED 2 = AMARELO. Depois grupos de 5 azul/verde; todo 10º é BRANCO.");
  await mostrar(frame((i) => {
    if (i === 0) return [255, 0, 0];
    if (i === 1) return [255, 160, 0];
    if ((i + 1) % 10 === 0) return [255, 255, 255];
    return Math.floor(i / 5) % 2 === 0 ? [0, 60, 255] : [0, 255, 60];
  }));
  await sleep(segundos * 1000);
} else if (modo === "grupos") {
  // Faixas de cor para descobrir quantos LEDs tem cada parte da fan.
  const faixas = [[6, [255, 0, 0]], [10, [0, 255, 0]], [14, [0, 60, 255]], [18, [255, 200, 0]], [Infinity, [180, 0, 255]]];
  console.log("1-6 VERMELHO | 7-10 VERDE | 11-14 AZUL | 15-18 AMARELO | 19+ ROXO");
  await mostrar(frame((i) => faixas.find(([fim]) => i < fim)[1]));
  await sleep(segundos * 1000);
} else if (modo === "cor") {
  const hex = args[1] && !args[1].startsWith("--") ? args[1] : "FFFFFF";
  const c = [0, 2, 4].map((o) => parseInt(hex.slice(o, o + 2), 16));
  // --de/--ate (1 a N, inclusive) acendem só uma faixa; o resto fica apagado.
  const de = Number(opt("de", 1)) - 1, ate = Number(opt("ate", leds)) - 1;
  await mostrar(frame((i) => (i >= de && i <= ate ? c : null)));
  await sleep(segundos * 1000);
} else if (modo === "arcoiris") {
  const inicio = performance.now();
  let quadros = 0;
  while (performance.now() - inicio < segundos * 1000) {
    const t0 = performance.now();
    const base = (t0 - inicio) / 1000 * 0.3;
    atual = frame((i) => hsv((base + i / leds) % 1));
    await send(atual);
    quadros++;
    const resta = 1000 / fps - (performance.now() - t0);
    if (resta > 0) await sleep(resta);
  }
  const s = (performance.now() - inicio) / 1000;
  console.log(`FPS médio: ${(quadros / s).toFixed(1)} (${quadros} quadros, ${leds} LEDs, ${baud} baud)`);
}

await sair();
