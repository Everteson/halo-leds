# Halo LEDs

Controle de LEDs endereçáveis (ARGB) do PC, feito em casa: fans e fita de LED
reagindo à **tela** (estilo ambilight), ao **som** do PC e a **efeitos**, com
um **Arduino** como controlador.

- **Tela:** a fita atrás do monitor acompanha a borda da imagem, por zonas. As fans pegam a cor dominante da cena.
- **Som:** os LEDs pulsam com os graves e as explosões do jogo, com a opção de juntar tela e som.
- **Efeitos:** mais de 15 efeitos (órbita, radar, ondas, fogo, aurora…) que usam a posição real de cada LED.
- **Várias saídas:** até 8 saídas, cada uma num pino do Arduino, cada uma com o seu modo (ex.: fans em Efeito e fita em Tela).
- Inicia com o Windows, fica na bandeja e religa as luzes sozinho.

> Feito para Windows 10/11. Não precisa de software do fabricante da placa-mãe.

## O que você precisa

| Peça | Observação |
|---|---|
| Arduino **UNO** ou **Nano** (clone com CH340 serve) | Ligado num USB do PC |
| LEDs **WS2812B** de 5 V (fita ou fans ARGB de 3 pinos) | Ordem de cor configurável |
| 1 resistor de **220–470 Ω** por saída | No fio de dados |
| Energia **5 V da fonte do PC** (Molex ou SATA) | Fios grossos (18–20 AWG); **nunca** jumper fino |

### Ligação de cada saída

```
Arduino Dx ──[220 Ω]── DIN dos LEDs      (x = 2 a 9)
Fonte 5V ───────────── +5V dos LEDs
Fonte GND ──┬───────── GND dos LEDs
            └───────── GND do Arduino   (terra em comum)
```

- O conector ARGB de 3 pinos é: **+5V, DADOS, (vazio), GND**. A seta no plástico marca o +5V.
- Confira com um **multímetro** qual fio da fonte é 5 V antes de ligar. 12 V queima os LEDs na hora.
- Não alimente muitos LEDs pelo 5 V do Arduino/USB.
- Fans com cabo "macho e fêmea" geralmente são **em paralelo**: todas repetem o mesmo sinal. Para controlar cada fan separado, ligue cada uma (ou cada grupo) num pino.

## Instalação

1. **Firmware do Arduino:** abra [`firmware/halo-adalight/halo-adalight.ino`](firmware/halo-adalight/halo-adalight.ino) no Arduino IDE, instale a biblioteca **FastLED** e grave como **Arduino UNO**. O protocolo está em [`PROTOCOLO.md`](firmware/halo-adalight/PROTOCOLO.md).
2. **App:** baixe o instalador na página de **Releases** deste repositório e instale. Ele é instalado só para o seu usuário, sem pedir administrador. Como o instalador não é assinado, o Windows pode mostrar "O Windows protegeu o computador": clique em **Mais informações → Executar assim mesmo**.
3. No app, em **Dispositivos**, cadastre as saídas (pino, tipo Cooler/Fita, quantidade de LEDs, ordem de cor) e use o **assistente de mapeamento** para marcar onde fica cada LED da fan.

Detalhes do app, dos modos e de como compilar: [`halo-studio/README.md`](halo-studio/README.md).

## Ferramenta de teste

[`tools/led-test`](tools/led-test) acende os LEDs direto pelo Arduino, sem o app: um LED por vez, faixas de cor, teste de carga, configuração das saídas. Serve para contar LEDs e conferir a ligação.

```bash
cd tools/led-test
npm install
node led-test.mjs andar --leds 30
```

## Compilar do código

Precisa de Rust (MSVC), Node.js e as Build Tools do Visual Studio.

```bash
cd halo-studio
npm install
npm run build
```

## Licença

[MIT](LICENSE)
