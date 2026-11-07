<p align="center">
  <img src="docs/banner.svg" alt="Halo LEDs" width="100%">
</p>

<p align="center">
  <a href="https://github.com/Everteson/halo-leds/releases"><img alt="Release" src="https://img.shields.io/github/v/release/Everteson/halo-leds?style=for-the-badge&color=C9A56B&labelColor=0A1119"></a>
  <img alt="Windows 10/11" src="https://img.shields.io/badge/Windows-10%20%7C%2011-0B2438?style=for-the-badge&logo=windows11&logoColor=E6EEF5&labelColor=0A1119">
  <img alt="Tauri 2" src="https://img.shields.io/badge/Tauri-2-0B2438?style=for-the-badge&logo=tauri&logoColor=E6EEF5&labelColor=0A1119">
  <img alt="Rust" src="https://img.shields.io/badge/Rust-stable-0B2438?style=for-the-badge&logo=rust&logoColor=E6EEF5&labelColor=0A1119">
  <img alt="Arduino" src="https://img.shields.io/badge/Arduino-UNO%20%7C%20Nano-0B2438?style=for-the-badge&logo=arduino&logoColor=E6EEF5&labelColor=0A1119">
  <a href="LICENSE"><img alt="Licença MIT" src="https://img.shields.io/badge/licen%C3%A7a-MIT-6BC49B?style=for-the-badge&labelColor=0A1119"></a>
</p>

<p align="center">
  Controle de LEDs endereçáveis (ARGB) do PC, sem software de fabricante.<br>
  Um <b>Arduino</b> de poucos reais vira o controlador; o app no Windows decide as cores, até 60 vezes por segundo.
</p>

<p align="center">
  <img src="docs/img/tela.png" alt="Modo Tela: a fita acompanha a borda do monitor" width="90%">
</p>

---

## Recursos

<table>
  <tr>
    <td width="33%" valign="top">
      <img src="docs/icons/tela.svg" width="28" alt=""><br>
      <b>Tela</b><br>
      A fita atrás do monitor acompanha a borda da imagem por zonas. As fans pegam a cor dominante da cena.
    </td>
    <td width="33%" valign="top">
      <img src="docs/icons/som.svg" width="28" alt=""><br>
      <b>Som</b><br>
      Os LEDs pulsam com os graves e as explosões. Dá para juntar com a tela: cor da cena, força do som.
    </td>
    <td width="33%" valign="top">
      <img src="docs/icons/efeitos.svg" width="28" alt=""><br>
      <b>Efeitos</b><br>
      Mais de 15 efeitos (órbita, radar, ondas, fogo, aurora, caleidoscópio) que usam a posição real de cada LED.
    </td>
  </tr>
  <tr>
    <td valign="top">
      <img src="docs/icons/saidas.svg" width="28" alt=""><br>
      <b>Várias saídas</b><br>
      Até 8 pinos no Arduino, cada um com tipo (Cooler ou Fita), quantidade de LEDs e ordem de cor próprios.
    </td>
    <td valign="top">
      <img src="docs/icons/cooler.svg" width="28" alt=""><br>
      <b>Todos os coolers juntos</b><br>
      Um efeito para todos os grupos de fans, sincronizado ou em cascata, ou ajustes separados por grupo.
    </td>
    <td valign="top">
      <img src="docs/icons/inicio.svg" width="28" alt=""><br>
      <b>Liga e esquece</b><br>
      Inicia com o Windows, fica na bandeja, acha o Arduino mesmo se mudar de porta USB e religa as luzes.
    </td>
  </tr>
</table>

## Telas

<table>
  <tr>
    <td width="50%"><img src="docs/img/efeitos.png" alt="Efeitos dos coolers"><br><sub><b>Efeitos</b>: todos os coolers juntos, com prévia na fan.</sub></td>
    <td width="50%"><img src="docs/img/audio.png" alt="Visualização de áudio"><br><sub><b>Áudio</b>: graves e agudos do que toca no PC.</sub></td>
  </tr>
  <tr>
    <td><img src="docs/img/dispositivos.png" alt="Saídas do Arduino"><br><sub><b>Dispositivos</b>: uma saída por pino, com limite de corrente.</sub></td>
    <td><img src="docs/img/mapeamento.png" alt="Assistente de mapeamento"><br><sub><b>Mapeamento</b>: clique onde cada LED acendeu.</sub></td>
  </tr>
</table>

## Como funciona

```mermaid
flowchart LR
    subgraph PC["PC com Windows"]
        T["Captura de tela<br/>60 FPS"] --> M["Motor de cores<br/>(Rust)"]
        S["Som do PC<br/>(loopback)"] --> M
        E["Efeitos"] --> M
    end
    M -- "USB serial · 1 Mbaud<br/>protocolo Adalight" --> A["Arduino UNO<br/>firmware halo-adalight"]
    A -- D6 --> F1["Cooler · grupo 1"]
    A -- D7 --> F2["Cooler · grupo 2"]
    A -- D9 --> FT["Fita do monitor"]
```

O app manda um quadro de cores por vez e o Arduino confirma cada um, então não há perda nem atraso acumulado. O protocolo está em [`PROTOCOLO.md`](firmware/halo-adalight/PROTOCOLO.md).

## Peças

| | Peça | Observação |
|---|---|---|
| <img src="docs/icons/chip.svg" width="20" alt=""> | Arduino **UNO** ou **Nano** | Clone com CH340 serve. Até 320 LEDs no total |
| <img src="docs/icons/fita.svg" width="20" alt=""> | LEDs **WS2812B** de 5 V | Fita ou fans ARGB de 3 pinos |
| <img src="docs/icons/teste.svg" width="20" alt=""> | 1 resistor de **220–470 Ω** por saída | No fio de dados |
| <img src="docs/icons/energia.svg" width="20" alt=""> | **5 V da fonte do PC** (Molex ou SATA) | Fio de 18–20 AWG; nada de jumper fino |

## Ligação

<p align="center">
  <img src="docs/referencias/ligacao-teste-1-fan-arduino-uno.png" alt="Ligação de uma fan no Arduino" width="520">
</p>

```
Arduino Dx ──[220 Ω]── DIN dos LEDs        (x = 2 a 9, um pino por saída)
Fonte 5V ───────────── +5V dos LEDs
Fonte GND ──┬───────── GND dos LEDs
            └───────── GND do Arduino     (terra em comum)
```

> [!IMPORTANT]
> Para mais de uma fan, os LEDs devem receber os 5 V **da fonte**, não do Arduino. O diagrama acima é o teste com uma fan só, no 5 V do Arduino.

> [!WARNING]
> Confira com um **multímetro** qual fio da fonte é 5 V antes de ligar. O Molex também tem **12 V**, que queima os LEDs na hora.

<details>
<summary><b>Dicas de ligação</b></summary>

- O conector ARGB de 3 pinos é **+5V, DADOS, (vazio), GND**. A seta no plástico marca o +5V.
- Na fita, o dado entra pela ponta de onde **sai a seta** (DIN).
- Fans com cabo "macho e fêmea" costumam ser **em paralelo**: todas repetem o mesmo sinal. Para controlar cada fan, ou grupo, separado, ligue cada um num pino.
- Fitas longas: ligue o 5 V nas duas pontas, para o branco não amarelar no fim.

</details>

## Instalação

<img src="docs/icons/baixar.svg" width="20" alt=""> **1. App:** baixe o instalador em [**Releases**](https://github.com/Everteson/halo-leds/releases) e instale. É só para o seu usuário e não pede administrador.

> [!NOTE]
> O instalador não é assinado. Se aparecer "O Windows protegeu o computador", clique em **Mais informações** → **Executar assim mesmo**.

<img src="docs/icons/chip.svg" width="20" alt=""> **2. Firmware:** abra [`firmware/halo-adalight/halo-adalight.ino`](firmware/halo-adalight/halo-adalight.ino) no Arduino IDE, instale a biblioteca **FastLED** e grave como **Arduino UNO**.

<img src="docs/icons/saidas.svg" width="20" alt=""> **3. Configurar:** no app, em **Dispositivos**, cadastre cada saída (pino, Cooler ou Fita, quantidade de LEDs, ordem de cor). Depois use o **assistente de mapeamento** para marcar onde fica cada LED da fan.

Detalhes de cada modo e de cada ajuste: [`halo-studio/README.md`](halo-studio/README.md).

## Ferramenta de teste

<img src="docs/icons/teste.svg" width="20" alt=""> [`tools/led-test`](tools/led-test) acende os LEDs direto pelo Arduino, sem o app. Serve para contar LEDs, conferir as cores, configurar as saídas e fazer teste de carga.

```bash
cd tools/led-test
npm install
node led-test.mjs andar --leds 30        # um LED por vez
node led-test.mjs config 6:11:GRB 9:113:GRB
node led-test.mjs info
```

## Compilar do código

<img src="docs/icons/codigo.svg" width="20" alt=""> Precisa de **Rust** (toolchain MSVC), **Node.js** e das **Build Tools do Visual Studio**.

```bash
cd halo-studio
npm install
npm run build        # gera o instalador em src-tauri/target/release/bundle/nsis
```

## Segurança

<img src="docs/icons/seguranca.svg" width="20" alt=""> Cada saída tem **limite de corrente** no app (padrão de 6 A), e o firmware tem um teto próprio. Mesmo assim:

- Use fios da grossura dos fios da fonte e prefira **solda** ou **borne** a encaixes de jumper.
- Fio **morno** com tudo em branco é normal; fio **quente** demais para segurar não é.
- Isole bem as emendas e prenda os cabos longe das fans.

## Licença

[MIT](LICENSE). Use, modifique e compartilhe à vontade.
