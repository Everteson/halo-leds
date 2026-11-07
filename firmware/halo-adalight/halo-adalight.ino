// Halo Studio — firmware do controlador ARGB (Arduino UNO/Nano, ATmega328P).
//
// v2: várias saídas de LED, cada uma num pino (D2–D9), com quantidade e ordem
// de cor próprias. A configuração chega pela serial e fica gravada na EEPROM.
// As cores chegam pelo protocolo Adalight (o mesmo do OpenRGB e do Halo Studio),
// em sequência pelas saídas na ordem configurada. Detalhes em PROTOCOLO.md.
//
// Ligação de cada saída: Dx -> resistor 220–470 Ω -> DIN dos LEDs; GND em comum.

#include <FastLED.h>
#include <EEPROM.h>

#define MAX_LEDS 320       // ~960 bytes de RAM; o UNO tem 2 KB no total
#define MAX_OUTPUTS 8
#define FIRST_PIN 2
#define LAST_PIN 9
#define NUM_PINS (LAST_PIN - FIRST_PIN + 1)
#define BAUD 1000000       // 16 MHz divide 1 Mbaud sem erro; o CH340 aguenta

// Teto de segurança estimado pelo FastLED (60 mA por pixel em branco total).
// Os LEDs usam os 5 V da fonte (Molex); o controle fino de consumo é do app.
// Atenção: as fans Acegeek acendem até 2 LEDs por pixel e estão em paralelo,
// então a corrente real pode ser bem maior que a estimada.
// Se os LEDs voltarem para o 5V do USB do Arduino, volte para 450.
#define MAX_MILLIAMPS 6000

// Sem dados por este tempo, os LEDs apagam (o PC desligou ou o app fechou).
#define IDLE_TIMEOUT_MS 5000UL

#define EEPROM_MAGIC 'H'
#define EEPROM_VERSION 2

// Ordem dos bytes que o LED espera: 0=RGB 1=RBG 2=GRB 3=GBR 4=BRG 5=BGR.
#define ORDER_GRB 2
#define NUM_ORDERS 6

struct Output {
  uint8_t pin;
  uint16_t count;
  uint8_t order;
};

CRGB leds[MAX_LEDS];
CLEDController *controllers[NUM_PINS];  // um por pino, índice = pino - FIRST_PIN
Output outputs[MAX_OUTPUTS];
uint8_t numOutputs = 0;
uint16_t totalLeds = 0;
unsigned long lastFrame = 0;
bool idle = true;

// Espera um byte por até `timeout` ms. Retorna -1 se não chegar.
static int readByte(unsigned long timeout) {
  unsigned long start = millis();
  while (!Serial.available()) {
    if (millis() - start > timeout) return -1;
  }
  return Serial.read();
}

// Os controladores trabalham em RGB "cru"; aqui os bytes já vão na ordem do LED.
static CRGB reorder(uint8_t r, uint8_t g, uint8_t b, uint8_t order) {
  switch (order) {
    case 1: return CRGB(r, b, g);
    case 2: return CRGB(g, r, b);
    case 3: return CRGB(g, b, r);
    case 4: return CRGB(b, r, g);
    case 5: return CRGB(b, g, r);
    default: return CRGB(r, g, b);
  }
}

// Valida uma configuração. Retorna NULL se estiver ok, ou o motivo do erro.
static const char *validate(const Output *outs, uint8_t n) {
  if (n < 1 || n > MAX_OUTPUTS) return "n";
  uint16_t total = 0;
  for (uint8_t i = 0; i < n; i++) {
    if (outs[i].pin < FIRST_PIN || outs[i].pin > LAST_PIN) return "pino";
    if (outs[i].order >= NUM_ORDERS) return "ordem";
    for (uint8_t j = 0; j < i; j++)
      if (outs[j].pin == outs[i].pin) return "repetido";
    total += outs[i].count;
    if (outs[i].count > MAX_LEDS || total > MAX_LEDS) return "total";
  }
  return NULL;
}

// Distribui o buffer `leds` entre os pinos, na ordem das saídas.
static void applyOutputs(const Output *outs, uint8_t n) {
  fill_solid(leds, MAX_LEDS, CRGB::Black);
  FastLED.show();  // apaga com a configuração antiga
  for (uint8_t p = 0; p < NUM_PINS; p++) controllers[p]->setLeds(leds, 0);
  uint16_t offset = 0;
  for (uint8_t i = 0; i < n; i++) {
    outputs[i] = outs[i];
    controllers[outs[i].pin - FIRST_PIN]->setLeds(leds + offset, outs[i].count);
    offset += outs[i].count;
  }
  numOutputs = n;
  totalLeds = offset;
  FastLED.show();
}

static void saveOutputs() {
  EEPROM.update(0, EEPROM_MAGIC);
  EEPROM.update(1, EEPROM_VERSION);
  EEPROM.update(2, numOutputs);
  for (uint8_t i = 0; i < numOutputs; i++) {
    int a = 3 + i * 4;
    EEPROM.update(a, outputs[i].pin);
    EEPROM.update(a + 1, outputs[i].count >> 8);
    EEPROM.update(a + 2, outputs[i].count & 0xFF);
    EEPROM.update(a + 3, outputs[i].order);
  }
}

// Lê a configuração gravada; sem ela, uma saída no D6 (compatível com o v1).
static void loadOutputs() {
  Output outs[MAX_OUTPUTS];
  uint8_t n = 0;
  if (EEPROM.read(0) == EEPROM_MAGIC && EEPROM.read(1) == EEPROM_VERSION) {
    n = EEPROM.read(2);
    if (n <= MAX_OUTPUTS) {
      for (uint8_t i = 0; i < n; i++) {
        int a = 3 + i * 4;
        outs[i].pin = EEPROM.read(a);
        outs[i].count = (uint16_t)EEPROM.read(a + 1) << 8 | EEPROM.read(a + 2);
        outs[i].order = EEPROM.read(a + 3);
      }
    }
  }
  if (validate(outs, n) != NULL) {
    outs[0] = {6, MAX_LEDS, ORDER_GRB};
    n = 1;
  }
  applyOutputs(outs, n);
}

// Acende uma sequência curta em cada saída ao ligar: confirma a ligação sem o PC.
static void bootTest() {
  for (uint8_t step = 0; step < 12; step++) {
    fill_solid(leds, MAX_LEDS, CRGB::Black);
    uint16_t offset = 0;
    for (uint8_t i = 0; i < numOutputs; i++) {
      if (step < outputs[i].count) leds[offset + step] = CHSV(step * 21, 255, 120);
      offset += outputs[i].count;
    }
    FastLED.show();
    delay(30);
  }
  fill_solid(leds, MAX_LEDS, CRGB::Black);
  FastLED.show();
}

static void printOutputs() {
  Serial.print(F("Hlo v2 max="));
  Serial.print(MAX_LEDS);
  Serial.print(F(" pinos=2-9 saidas="));
  for (uint8_t i = 0; i < numOutputs; i++) {
    if (i) Serial.print(',');
    Serial.print(outputs[i].pin);
    Serial.print(':');
    Serial.print(outputs[i].count);
    Serial.print(':');
    Serial.print(outputs[i].order);
  }
  Serial.print('\n');
}

// "Hlo" já lido; trata 'C' (configurar) e '?' (consultar).
static void handleCommand() {
  int cmd = readByte(50);
  if (cmd == '?') {
    printOutputs();
    return;
  }
  if (cmd != 'C') return;

  int n = readByte(50);
  if (n < 1 || n > MAX_OUTPUTS) {
    Serial.print(F("Hlo ERR n\n"));
    return;
  }
  Output outs[MAX_OUTPUTS];
  uint8_t chk = n;
  for (uint8_t i = 0; i < n; i++) {
    int pin = readByte(50), hi = readByte(50), lo = readByte(50), order = readByte(50);
    if (order < 0) {
      Serial.print(F("Hlo ERR n\n"));
      return;
    }
    outs[i] = {(uint8_t)pin, (uint16_t)(hi << 8 | lo), (uint8_t)order};
    chk ^= pin ^ hi ^ lo ^ order;
  }
  if (readByte(50) != (chk ^ 0x55)) {
    Serial.print(F("Hlo ERR chk\n"));
    return;
  }
  const char *err = validate(outs, n);
  if (err) {
    Serial.print(F("Hlo ERR "));
    Serial.print(err);
    Serial.print('\n');
    return;
  }
  applyOutputs(outs, n);
  saveOutputs();
  Serial.print(F("Hlo OK\n"));
}

// "Ada" já lido; lê o quadro e distribui pelas saídas.
static void handleFrame() {
  int hi = readByte(50), lo = readByte(50), chk = readByte(50);
  if (hi < 0 || lo < 0 || chk < 0 || chk != (hi ^ lo ^ 0x55)) return;

  // A 1 Mbaud chega um byte a cada 10 µs, e o buffer da serial tem só 64 bytes.
  // Por isso o laço só copia bytes crus, sem chamar millis() a cada byte;
  // a conversão de ordem de cor fica para depois do quadro inteiro.
  uint16_t count = ((uint16_t)hi << 8 | lo) + 1;
  uint16_t bytes = count * 3;
  uint16_t keep = (count < totalLeds ? count : totalLeds) * 3;
  uint8_t *dst = (uint8_t *)leds;
  uint16_t got = 0;
  unsigned long start = millis();
  while (got < bytes) {
    int c = Serial.read();
    if (c >= 0) {
      if (got < keep) dst[got] = c;
      got++;
    } else if (millis() - start > 200) {
      return;  // quadro incompleto: descarta
    }
  }

  uint16_t offset = 0;
  for (uint8_t o = 0; o < numOutputs; o++) {
    uint16_t end = offset + outputs[o].count;
    if (end > count) end = count;
    if (outputs[o].order != 0)
      for (uint16_t i = offset; i < end; i++)
        leds[i] = reorder(leds[i].r, leds[i].g, leds[i].b, outputs[o].order);
    offset += outputs[o].count;
  }
  // Saídas que o quadro não cobriu ficam apagadas.
  for (uint16_t i = count; i < totalLeds; i++) leds[i] = CRGB::Black;

  FastLED.show();
  Serial.write(0x06);  // ACK: quadro mostrado; o app pode mandar o próximo
  lastFrame = millis();
  idle = false;
}

#define ADD_PIN(p) controllers[(p) - FIRST_PIN] = &FastLED.addLeds<WS2812B, p, RGB>(leds, 0)

void setup() {
  ADD_PIN(2);
  ADD_PIN(3);
  ADD_PIN(4);
  ADD_PIN(5);
  ADD_PIN(6);
  ADD_PIN(7);
  ADD_PIN(8);
  ADD_PIN(9);
  FastLED.setMaxPowerInVoltsAndMilliamps(5, MAX_MILLIAMPS);
  FastLED.setDither(0);
  loadOutputs();
  bootTest();
  Serial.begin(BAUD);
  Serial.print("Ada\n");  // anuncia para quem procura dispositivos Adalight
}

void loop() {
  if (Serial.available()) {
    // Procura um cabeçalho byte a byte, ressincronizando se algo se perder.
    int c = Serial.read();
    if (c == 'A') {
      if (readByte(50) == 'd' && readByte(50) == 'a') handleFrame();
    } else if (c == 'H') {
      if (readByte(50) == 'l' && readByte(50) == 'o') handleCommand();
    }
    return;
  }

  if (!idle && millis() - lastFrame > IDLE_TIMEOUT_MS) {
    fill_solid(leds, MAX_LEDS, CRGB::Black);
    FastLED.show();
    idle = true;
  }
  if (idle && millis() - lastFrame > 1000) {
    Serial.print("Ada\n");  // reanuncia a cada segundo enquanto ninguém fala
    lastFrame = millis();
  }
}
