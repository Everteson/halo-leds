<#
.SYNOPSIS
  Testa e mapeia os LEDs ligados no Arduino (firmware halo-adalight).

.EXAMPLE
  .\led-test.ps1 -Modo contar                # numera os LEDs por cor para contar
  .\led-test.ps1 -Modo andar -Passo          # acende um LED por vez, Enter avança
  .\led-test.ps1 -Modo cor -Cor FF0000       # tudo vermelho
  .\led-test.ps1 -Modo arcoiris -Segundos 10 # animação a 60 FPS, mede o FPS real
  .\led-test.ps1 -Modo apagar
#>
param(
  [ValidateSet('contar', 'andar', 'cor', 'arcoiris', 'apagar')]
  [string]$Modo = 'contar',
  [string]$Porta = 'COM3',
  [int]$Baud = 1000000,
  [int]$Leds = 60,
  [string]$Cor = 'FFFFFF',
  # 0 a 100. Baixo por padrão: no teste os LEDs puxam do USB.
  [int]$Brilho = 55,
  [int]$Segundos = 30,
  # andar: tempo em cada LED (ms)
  [int]$Intervalo = 700,
  [int]$Fps = 60,
  # andar: espera Enter em cada LED
  [switch]$Passo
)

$ErrorActionPreference = 'Stop'
$k = [Math]::Max(0, [Math]::Min(100, $Brilho)) / 100.0

function New-Frame([int]$n) { New-Object byte[] ($n * 3) }

function Set-Led([byte[]]$frame, [int]$i, [int]$r, [int]$g, [int]$b) {
  $frame[$i * 3] = [byte]($r * $k)
  $frame[$i * 3 + 1] = [byte]($g * $k)
  $frame[$i * 3 + 2] = [byte]($b * $k)
}

function Send-Frame([System.IO.Ports.SerialPort]$port, [byte[]]$rgb) {
  $n = $rgb.Length / 3 - 1
  $hi = ($n -shr 8) -band 0xFF
  $lo = $n -band 0xFF
  $head = [byte[]](0x41, 0x64, 0x61, $hi, $lo, ($hi -bxor $lo -bxor 0x55))
  $port.Write($head, 0, $head.Length)
  $port.Write($rgb, 0, $rgb.Length)
}

function Hsv([double]$h) {
  $f = { param($n) $x = ($n + $h * 6) % 6; [int](255 * (1 - [Math]::Max(0, [Math]::Min(1, [Math]::Min($x, 4 - $x))))) }
  , @((& $f 5), (& $f 3), (& $f 1))
}

$port = New-Object System.IO.Ports.SerialPort $Porta, $Baud, 'None', 8, 'One'
$port.WriteTimeout = 2000
$port.Open()
# Abrir a porta reinicia o Arduino; espera o bootloader e o teste de boot.
Start-Sleep -Milliseconds 2200
$port.DiscardInBuffer()

try {
  switch ($Modo) {
    'contar' {
      # LED 0 vermelho e LED 1 amarelo mostram o começo e o sentido.
      # Depois, grupos de 5 alternando azul/verde; todo 10º é branco.
      $frame = New-Frame $Leds
      for ($i = 0; $i -lt $Leds; $i++) {
        if ($i -eq 0) { Set-Led $frame $i 255 0 0 }
        elseif ($i -eq 1) { Set-Led $frame $i 255 160 0 }
        elseif (($i + 1) % 10 -eq 0) { Set-Led $frame $i 255 255 255 }
        elseif ([Math]::Floor($i / 5) % 2 -eq 0) { Set-Led $frame $i 0 60 255 }
        else { Set-Led $frame $i 0 255 60 }
      }
      Write-Host "LED 1 = VERMELHO, LED 2 = AMARELO (sentido)."
      Write-Host "Depois grupos de 5: azul, verde, azul... e todo 10º LED é BRANCO."
      Write-Host "Conte quantos acendem na fan. Segurando por $Segundos s..."
      $fim = (Get-Date).AddSeconds($Segundos)
      while ((Get-Date) -lt $fim) { Send-Frame $port $frame; Start-Sleep -Milliseconds 500 }
    }
    'andar' {
      for ($i = 0; $i -lt $Leds; $i++) {
        $frame = New-Frame $Leds
        Set-Led $frame $i 255 255 255
        Send-Frame $port $frame
        if ($Passo) { Read-Host "LED $($i + 1) aceso (Enter = próximo, Ctrl+C = sair)" | Out-Null }
        else { Write-Host "LED $($i + 1)"; Start-Sleep -Milliseconds $Intervalo }
      }
    }
    'cor' {
      $r = [Convert]::ToInt32($Cor.Substring(0, 2), 16)
      $g = [Convert]::ToInt32($Cor.Substring(2, 2), 16)
      $b = [Convert]::ToInt32($Cor.Substring(4, 2), 16)
      $frame = New-Frame $Leds
      for ($i = 0; $i -lt $Leds; $i++) { Set-Led $frame $i $r $g $b }
      $fim = (Get-Date).AddSeconds($Segundos)
      while ((Get-Date) -lt $fim) { Send-Frame $port $frame; Start-Sleep -Milliseconds 500 }
    }
    'arcoiris' {
      $periodo = [TimeSpan]::FromSeconds(1.0 / $Fps)
      $sw = [Diagnostics.Stopwatch]::StartNew()
      $quadros = 0
      while ($sw.Elapsed.TotalSeconds -lt $Segundos) {
        $t0 = $sw.Elapsed
        $frame = New-Frame $Leds
        $base = $sw.Elapsed.TotalSeconds * 0.3
        for ($i = 0; $i -lt $Leds; $i++) {
          $c = Hsv ((($base + $i / $Leds) % 1.0))
          Set-Led $frame $i $c[0] $c[1] $c[2]
        }
        Send-Frame $port $frame
        $quadros++
        $resta = $periodo - ($sw.Elapsed - $t0)
        if ($resta -gt [TimeSpan]::Zero) { Start-Sleep -Milliseconds ([int]$resta.TotalMilliseconds) }
      }
      Write-Host ("FPS medio: {0:N1} ({1} quadros, {2} LEDs, {3} baud)" -f ($quadros / $sw.Elapsed.TotalSeconds), $quadros, $Leds, $Baud)
    }
  }
}
finally {
  Send-Frame $port (New-Frame $Leds)
  $port.Close()
}
