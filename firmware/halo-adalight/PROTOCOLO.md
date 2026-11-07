# Protocolo serial do firmware Halo (v2)

Serial a **1.000.000 baud**, 8N1. Abrir a porta reinicia o Arduino: espere o
`Ada\n` que ele manda ao terminar o boot (até ~2,5 s) antes de enviar algo.

## Saídas

O Arduino tem **várias saídas** de LED (pinos digitais **D2 a D9**). Cada saída tem:

- **pino** (2–9)
- **quantidade** de LEDs (0–320; a soma de todas as saídas é no máximo 320)
- **ordem de cor** do LED: `0=RGB 1=RBG 2=GRB 3=GBR 4=BRG 5=BGR` (WS2812B = GRB)

A configuração fica gravada na EEPROM e vale nos próximos boots. Sem configuração
gravada, o padrão é **uma saída no D6 com 320 LEDs em GRB** (compatível com o firmware v1).

## Quadro de cores (Adalight)

```
'A' 'd' 'a'  hi  lo  (hi ^ lo ^ 0x55)   e então N = (hi<<8 | lo) + 1 LEDs em R G B
```

Os LEDs vão **em sequência pelas saídas, na ordem em que foram configuradas**:
os primeiros `quantidade[0]` vão para a saída 0, os próximos para a saída 1, e
assim por diante. Mande sempre R, G, B: o firmware converte para a ordem de cor
de cada saída. Se o quadro tiver menos LEDs que o total, o resto apaga; se tiver
mais, o excesso é ignorado.

Depois de mostrar cada quadro, o Arduino responde **1 byte `0x06` (ACK)**. Use para
cadenciar: mande o próximo quadro só depois do ACK (ou de um tempo limite de
~50 ms). Durante o `show()` as interrupções ficam desligadas por ~30 µs por LED
(≈9 ms com 311 LEDs), e bytes que chegam nesse intervalo se perdem; por isso não
mande quadros em cima um do outro. Quadros incompletos são descartados.

Sem quadros por 5 s, todos os LEDs apagam. Parado, o Arduino reanuncia `Ada\n` a cada 1 s.

## Configurar saídas

```
'H' 'l' 'o'  'C'  n  [pino  qtd_hi  qtd_lo  ordem] × n  chk
```

- `n` = número de saídas (1–8)
- `chk` = XOR de todos os bytes depois de `'C'` (de `n` até o último byte da última saída) XOR `0x55`

Resposta (texto, termina em `\n`):

- `Hlo OK\n` — aplicado e gravado
- `Hlo ERR <motivo>\n` — nada mudou. Motivos: `chk`, `n`, `pino`, `repetido`, `total`, `ordem`

## Consultar

```
'H' 'l' 'o' '?'
```

Resposta, uma linha:

```
Hlo v2 max=320 pinos=2-9 saidas=6:11:2,9:120:2\n
```

(`saidas` = lista `pino:quantidade:ordem` na ordem do quadro.)

## Limite de corrente

O firmware tem um teto de segurança (`MAX_MILLIAMPS`, estimativa do FastLED por
pixel). O controle fino de consumo é do app.
