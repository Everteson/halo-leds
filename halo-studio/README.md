# Halo Studio

App de LEDs do PC: a fita acompanha a **tela** (60 FPS), a **música** ou um **efeito**.
Tauri 2 + Rust, interface em HTML/CSS/JS puro (sem bundler).

## Rodar

```bash
npm install
npm run dev        # abre o app (compila o Rust em modo debug)
npm run build      # instalador NSIS em src-tauri/target/release/bundle
npm run preview    # só a interface no navegador, com backend falso: http://localhost:5174
```

Testes do backend (dentro de `src-tauri`):

```bash
cargo test --release                                    # testes rápidos
cargo test --release laco_tela -- --ignored --nocapture # captura real + saída simulada, 10 s
cargo test --release laco_musica -- --ignored --nocapture
```

## Versão instalada, bandeja e início com o Windows

`npm.cmd run build` gera `src-tauri/target/release/bundle/nsis/Halo Studio_0.1.0_x64-setup.exe`. O instalador é
**só para o usuário** (sem admin), instala em `%LOCALAPPDATA%\Halo Studio` e cria o atalho no Menu Iniciar.

**Instale pelo Explorer:** dê dois cliques em `instalar-halo.bat` (nesta pasta). Ele instala por cima,
liga o início com o Windows, copia as configurações de `dev/config-do-halo.json` (se ainda não houver
nenhuma) e abre o Halo na bandeja. Não rode o instalador de dentro do Claude (terminal, `npm run dev`):
o app Claude é empacotado e o Windows redireciona o que ele grava no AppData para
`%LOCALAPPDATA%\Packages\Claude_…\LocalCache`, uma cópia que o Windows não usa no login.

- **Bandeja:** o ícone perto do relógio tem *Abrir*, *Ligar/Pausar as luzes* e *Sair (apaga os LEDs)*. O X da
  janela só esconde. Abrir o app de novo traz a janela que já está aberta (só roda um Halo por vez).
- **Iniciar com o Windows:** a chave `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, valor `Halo Studio`,
  aponta para o exe instalado com `--autostart`. Ela é ligada sozinha na **primeira vez que a versão instalada
  abre**, e dá para desligar em Configurações → Iniciar com o Windows. A versão de `npm run dev` não registra.
- **No boot:** abre só na bandeja. Se as luzes estavam ligadas (o Halo guarda o último Iniciar/Parar), liga
  de novo, tentando por até 30 s enquanto o Arduino não aparece. O andamento aparece no menu do ícone.
- **Porta COM:** se a configurada sumiu (o Arduino trocou de USB), o Halo procura o Arduino pelo chip (CH340
  1A86, Arduino 2341/2A03, FTDI 0403) e atualiza a configuração.
- **Tela + Som (Cooler) e Reagir ao som:** na tela Tela, cartão do som. **Força do grave** (0–300%, padrão
  150%) multiplica o nível do grave antes de virar brilho. **Cor sempre viva** (fans, ligada por padrão) usa
  só o matiz da cor da tela no brilho máximo, e cena cinza ou preta vira branco quente: assim, em jogo escuro, o
  brilho vem do som. Também há **Sensibilidade**, **Brilho mínimo** e **Faixa** (graves ou tudo).

Para conferir o registro:

```
reg query HKCU\Software\Microsoft\Windows\CurrentVersion\Run /v "Halo Studio"
```

## Como funciona

| Arquivo | O que faz |
|---|---|
| `src-tauri/src/capture.rs` | DXGI Desktop Duplication. O quadro é copiado para uma textura com mipmaps na GPU, `GenerateMips` reduz, e só o nível de ~320×180 vem para a CPU. |
| `src-tauri/src/zones.rs` | Zonas em volta da borda (sentido horário a partir do canto inferior esquerdo), cor média de cada zona, recorte da barra de tarefas e das faixas pretas, mapeamento zona→LED. |
| `src-tauri/src/engine.rs` | Laço a 60 Hz com relógio fixo: fonte (tela/música/efeito), suavização, ajustes e envio às saídas. |
| `src-tauri/src/color.rs` | Saturação, temperatura de cor, brilho, gama e limite de corrente (estimativa de 20 mA por canal). |
| `src-tauri/src/audio.rs` | Loopback WASAPI (cpal), graves/agudos e FFT de 2048 pontos em 24 faixas. |
| `src-tauri/src/visuals.rs` | Modos de música (Pulso, Espectro, Energia, Ondas) e efeitos. |
| `src-tauri/src/output.rs` | Trait `LedOutput` com três backends: `Adalight` (serial), `SerialLog` (simulação) e `OpenRgbDirect`. |
| `src-tauri/src/openrgb.rs` | Cópia enxuta do cliente SDK de `app/`: só lê dispositivos e envia LED a LED. |

A configuração fica em `%APPDATA%\com.evert.halostudio\config.json` e é salva a cada mudança.

## Ligar o Arduino

1. Grave `firmware/halo-adalight` (v2) no UNO. Protocolo em `firmware/halo-adalight/PROTOCOLO.md`.
2. Em **Dispositivos → Arduino**: escolha a porta COM (o CH340 aparece marcado como "Arduino?") e o baud **1000000**.
3. Em **Dispositivos → Saídas**, monte a lista (veja abaixo) e clique em **Enviar saídas**. O app manda
   o `HloC` e mostra a resposta (`Hlo OK` ou `Hlo ERR …`). Com o Halo rodando, mudar um pino,
   uma quantidade ou a ordem de cor reenvia sozinho.
4. Em cada saída, **Testar esta saída** abre a porta por ~5 s e acende só ela: vermelho, verde, azul e
   um ponto correndo do primeiro LED (dourado) até o último.
5. Desligue **Simular saída** e clique em **Iniciar Halo**. Com a simulação ligada, nada vai pela porta.

Quadro: `'A' 'd' 'a' hi lo (hi^lo^0x55)` com `hi:lo = LEDs − 1`, seguido de R G B por LED. Os LEDs
das saídas vão em sequência, na ordem da lista. Se o Arduino não responder ao `Hlo?` (firmware v1),
tudo vai para o D6, como antes.

## Saídas

Cada saída é um pino do Arduino com uma **Fan** (cooler) ou uma **Fita**. O firmware v2 aceita os pinos
**D2–D9**, até 8 saídas e **320 LEDs** no total. O app lê a faixa de pinos e o máximo da resposta ao `Hlo?`
(`pinos=2-9 max=320`). Cada saída tem:

- **Pino**, **pixels** e **ordem de cor** (GRB no WS2812B). O app avisa quando um pino se repete.
- **Modo próprio.** Na Fita: Tela (zonas), Efeito, Som ou Desligada. Na Fan: Efeito, Cor da tela (cor
  dominante, mudando em ~0,4 s), Som ou Tela + Som. Saídas diferentes rodam modos diferentes ao mesmo
  tempo, por exemplo a fita em Tela e as fans em Som. O modo **Volume no espaço** acende o anel com os
  graves e a moldura com os agudos.
- **Reagir ao som** (opcional, desligado por padrão), na tela Tela, para a Fita em Tela (zonas) e o Cooler em
  Cor da tela. As cores continuam as da tela e o brilho pulsa com o som: sobe na hora com a batida e cai
  em ~0,3 s. Ajustes: **Intensidade** (0 = nada), **Brilho mínimo** (no silêncio) e **Faixa** (graves ou
  tudo). O limite de consumo vale também no pico.
- **Brilho** (multiplica o global) e **limite de consumo**, que vem ligado em 6 A em toda saída nova (validado com a fita de 300 LEDs na fonte via Molex).
  A estimativa é de 20 mA por canal × pixels × **fans em paralelo** × **LEDs por pixel**. As 7 fans
  com 2 LEDs por pixel dão 9,24 A em branco. Sem limite, o editor mostra um aviso em vermelho.
  O **Limite total** (tela Tela → Brilho e Cor) corta todas as saídas juntas.
- **Mapeamento próprio**: o assistente de pontos na Fan, e as zonas e o "Primeiro LED na zona" na Fita.
  As zonas da fita são ajustadas na tela **Tela**, com a fita escolhida no cartão **Saída**.

**Todos os coolers / Todas as fitas.** Com mais de uma saída do mesmo tipo, a aba começa em **Todos**:
modo, efeito, cores, velocidade, som e brilho valem para todas ao mesmo tempo (`follow`). Escolher um
grupo e mexer nele dá a ele **ajustes próprios**, e o "Todos" deixa esse grupo como está, a não ser que se
ligue **Aplicar também nos grupos com ajustes próprios**. O botão **Voltar a seguir Todos** copia os ajustes do
grupo de volta. Na tela Efeitos, o cartão **Cascata entre grupos** tem **Defasar entre grupos** (Juntos =
todos iguais; subindo, o efeito corre de um grupo para o outro) e a ordem (ex.: frente → topo → traseira).

Nas telas Tela, Áudio e Efeitos há duas abas: **Cooler (CPU)**, com as fans, e **Fita (Monitor)**, com
as fitas. Cada aba mostra a prévia e os ajustes (efeito, cores, velocidade, brilho) só das saídas daquele
tipo. Se houver mais de uma saída do mesmo tipo, elas aparecem como botões dentro da aba. Escolher um
efeito ou uma visualização põe aquela saída nesse modo.

**Controle de fluxo:** o firmware v2 manda o byte `0x06` (ACK) depois de mostrar cada quadro. O app só
manda o próximo quadro depois do ACK ou de 50 ms, porque durante o `show()` (~30 µs por LED) o
Arduino perde os bytes que chegam. Com 311 LEDs, isso dá uns 50 FPS reais, e é esse FPS que aparece na
tela. Se o firmware não mandar ACK em 3 quadros seguidos (v1), o app para de esperar.

Ao abrir uma configuração antiga (uma fita só), o app migra para **uma saída Fan no D6 com 11 pixels**,
7 fans em paralelo e limite de 6 A, mantendo o mapa e o efeito. O limite total começa em 12 A.

**Exemplo: fan no D6 e fita no D9**
1. A fan migrada já está no D6. Confira **Fans em paralelo** (7) e **LEDs por pixel** (2).
2. Clique em **+ Fita de LED**: ela entra no próximo pino livre (D9). Ajuste **Pixels**.
3. Clique em **Enviar saídas** e veja "Hlo OK".
4. Em **Testar esta saída**, só a fita acende. Ajuste o "Primeiro LED na zona" e o sentido.
5. Escolha os modos (fita em Tela, fans em Som, por exemplo) e clique em **Iniciar Halo**.

Só um programa por vez abre a COM: com o `tools/led-test` aberto, o Halo avisa que a porta está em uso.
Enquanto roda, o Halo manda quadro a 60 Hz mesmo com a tela parada, então o desligamento por
inatividade do firmware (5 s) só acontece depois de parar. O `thread::sleep` do Rust usa timer de alta
resolução no Windows: o teste `laco_tela` mede 60,0 FPS.

**Fans Acegeek:** 11 pixels endereçáveis por fan; cada pixel acende mais de um LED físico (anel e
borda, por causa do espelho infinito) e a ordem não é um círculo simples (ver
`tools/led-test/mapa-fan-acegeek.json`). A fan N começa no pixel N×11. Para a fan mostrar uma zona da
tela, use o mapeamento manual, por exemplo 11 vezes o número da zona do meio da borda de cima.

## Assistente de mapeamento

Em **Dispositivos**, escolha uma saída Fan e clique em **Mapear LEDs**. Diga quantas fans há em
sequência no mesmo fio e o total de pixels (11 por fan). O assistente abre a COM, acende um pixel por vez em vermelho, e você
clica no desenho onde ele acendeu. O botão direito (ou Shift + clique) marca um 2º ponto do mesmo pixel.

- Atalhos: **S** pula ("não acendeu nada"), **Backspace** volta, **R** refaz, **Enter** conclui e **Esc** fecha.
- No fim, uma onda de arco-íris gira na fan e no desenho para você conferir. **Salvar mapa** grava
  `leds.points` (x/y de 0 a 1 por pixel), `leds.shape`, `leds.fans` e `leds.count` no config.json.
- Com mapa, os efeitos e a música usam o ângulo e a distância de cada pixel até o centro da fan
  (`visuals::positions`). Sem mapa, continuam pela ordem. **Apagar mapa** volta ao comportamento antigo.
- A COM fica aberta só enquanto o assistente está aberto, e só a saída mapeada acende. Enquanto isso, Iniciar e Testar recusam.

## Efeitos

São 18 efeitos em `visuals::render_effect`. Cada um recebe a posição do pixel (`visuals::Pos`): ângulo `p`,
raio `d` (abaixo de 0,62 é o anel interno, acima é a moldura), x/y locais da fan e o índice da fan.

| Efeito | O que faz | "Tamanho" vira |
|---|---|---|
| Estático, Respiração, Arco-íris, Onda, Gradiente, Cometa | os originais, agora pelo mapa | rastro (Cometa) |
| Órbita dupla | anel num sentido (cor 1), moldura no outro (cor 2) | rastro |
| Ondas na água | anéis saindo do centro pelo raio | largura |
| Radar | feixe girando com rastro | rastro |
| Gangorra | os lados trocam de cor; o eixo gira devagar | suavidade (+ Ângulo) |
| Varredura | faixa indo e voltando | largura (+ Ângulo) |
| Yin-yang | duas metades girando | suavidade |
| Fogo | quente embaixo, chamas subindo (ruído) | intensidade |
| Aurora | cores 1/2 fluindo com ruído suave | contraste |
| Céu estrelado | brilhos com ritmo fixo por pixel (hash com semente) | densidade |
| Batimento | pulso duplo saindo do centro | duração |
| Espiral | arco-íris pelo ângulo + raio | torção |
| Caleidoscópio | simetria espelhada girando | número de espelhos |

Com várias fans, cada uma usa as próprias coordenadas, e **Defasar fans** (0 = juntas) atrasa
cada fan. O brilho global e o limite de consumo continuam valendo. Com o Halo parado, a tela
Efeitos mostra uma prévia calculada pelo backend (`effect_preview`), sem mandar nada aos LEDs.

## OpenRGB

Opcional. Só recebem quadros os dispositivos com modo **Direct sem a flag de gravação automática**.
A placa ASRock e o teclado Redragon gravam na memória a cada atualização, então ficam bloqueados.
Listar os dispositivos é só leitura.
