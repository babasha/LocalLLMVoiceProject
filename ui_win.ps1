#requires -version 5
# Lightweight desktop UI for the voice translator.
# Launches voice-translator.exe (--mode full, VOICE_UI=1), tails its output
# file and shows live RU -> EN with a running history.

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()

# --- config ---
$proj    = "C:\Users\egorb\OneDrive\Documentos\GitHub\LocalLLMVoiceProject"
$exe     = "C:\cargo-target\voice-translator\release\voice-translator.exe"
$cudaBin = "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3\bin\x64"
$cudaBin2= "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3\bin"
$uiFile  = Join-Path $env:TEMP "voice_ui_stream.txt"
$errFile = Join-Path $env:TEMP "voice_ui_err.txt"
Set-Content -Path $uiFile -Value "" -Encoding UTF8
Set-Content -Path $errFile -Value "" -Encoding UTF8

# --- colors ---
$bg     = [System.Drawing.Color]::FromArgb(24,26,32)
$card   = [System.Drawing.Color]::FromArgb(34,37,46)
$fgDim  = [System.Drawing.Color]::FromArgb(150,156,170)
$fgRu   = [System.Drawing.Color]::FromArgb(232,234,240)
$fgEn   = [System.Drawing.Color]::FromArgb(120,200,255)
$accent = [System.Drawing.Color]::FromArgb(90,180,120)

# --- form ---
$form = New-Object System.Windows.Forms.Form
$form.Text = "Voice Translator  -  RU to EN (local, GPU)"
$form.Size = New-Object System.Drawing.Size(760,560)
$form.StartPosition = "CenterScreen"
$form.BackColor = $bg
$form.Font = New-Object System.Drawing.Font("Segoe UI",10)

# status
$status = New-Object System.Windows.Forms.Label
$status.Text = "Loading models (~10s), then speak Russian into the mic..."
$status.ForeColor = $fgDim
$status.AutoSize = $true
$status.Location = New-Object System.Drawing.Point(20,14)
$form.Controls.Add($status)

# RU card
$ruTag = New-Object System.Windows.Forms.Label
$ruTag.Text = "RU  (speech)"; $ruTag.ForeColor = $fgDim; $ruTag.AutoSize = $true
$ruTag.Location = New-Object System.Drawing.Point(22,46)
$form.Controls.Add($ruTag)

$ruBox = New-Object System.Windows.Forms.Label
$ruBox.BackColor = $card; $ruBox.ForeColor = $fgRu
$ruBox.Font = New-Object System.Drawing.Font("Segoe UI",17)
$ruBox.Location = New-Object System.Drawing.Point(20,68)
$ruBox.Size = New-Object System.Drawing.Size(700,90)
$ruBox.Padding = New-Object System.Windows.Forms.Padding(12)
$ruBox.TextAlign = "TopLeft"
$form.Controls.Add($ruBox)

# EN card
$enTag = New-Object System.Windows.Forms.Label
$enTag.Text = "EN  (translation)"; $enTag.ForeColor = $fgDim; $enTag.AutoSize = $true
$enTag.Location = New-Object System.Drawing.Point(22,170)
$form.Controls.Add($enTag)

$enBox = New-Object System.Windows.Forms.Label
$enBox.BackColor = $card; $enBox.ForeColor = $fgEn
$enBox.Font = New-Object System.Drawing.Font("Segoe UI",20,[System.Drawing.FontStyle]::Bold)
$enBox.Location = New-Object System.Drawing.Point(20,192)
$enBox.Size = New-Object System.Drawing.Size(700,100)
$enBox.Padding = New-Object System.Windows.Forms.Padding(12)
$enBox.TextAlign = "TopLeft"
$form.Controls.Add($enBox)

# history
$histTag = New-Object System.Windows.Forms.Label
$histTag.Text = "History"; $histTag.ForeColor = $fgDim; $histTag.AutoSize = $true
$histTag.Location = New-Object System.Drawing.Point(22,304)
$form.Controls.Add($histTag)

$hist = New-Object System.Windows.Forms.TextBox
$hist.Multiline = $true; $hist.ReadOnly = $true; $hist.ScrollBars = "Vertical"
$hist.BackColor = $card; $hist.ForeColor = $fgRu
$hist.Font = New-Object System.Drawing.Font("Consolas",10)
$hist.Location = New-Object System.Drawing.Point(20,326)
$hist.Size = New-Object System.Drawing.Size(700,140)
$hist.Anchor = "Top,Bottom,Left,Right"
$form.Controls.Add($hist)

# buttons
$btnClear = New-Object System.Windows.Forms.Button
$btnClear.Text = "Clear"; $btnClear.Location = New-Object System.Drawing.Point(540,476)
$btnClear.Size = New-Object System.Drawing.Size(85,30)
$btnClear.Anchor = "Bottom,Right"
$btnClear.Add_Click({ $hist.Clear() })
$form.Controls.Add($btnClear)

$btnStop = New-Object System.Windows.Forms.Button
$btnStop.Text = "Stop"; $btnStop.Location = New-Object System.Drawing.Point(635,476)
$btnStop.Size = New-Object System.Drawing.Size(85,30)
$btnStop.Anchor = "Bottom,Right"
$btnStop.Add_Click({ $form.Close() })
$form.Controls.Add($btnStop)

# --- launch the translator ---
# _ui_launch.bat sets env + redirects the child's stdout/stderr straight to the
# temp files (OS-level), so there is no PowerShell event pumping to stall under
# ShowDialog. The GUI just tails those files.
$launcher = Join-Path $proj "_ui_launch.bat"
$proc = Start-Process -FilePath $launcher -WorkingDirectory $proj -WindowStyle Hidden -PassThru

# --- tail reader ---
$us = [char]0x1f
$fsUi  = [System.IO.FileStream]::new($uiFile, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
$rdUi  = [System.IO.StreamReader]::new($fsUi, [System.Text.Encoding]::UTF8)
$fsErr = [System.IO.FileStream]::new($errFile,[System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
$rdErr = [System.IO.StreamReader]::new($fsErr,[System.Text.Encoding]::UTF8)
$script:ready = $false

$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 120
$timer.Add_Tick({
  # UI records from stdout (info/readiness logs are suppressed via RUST_LOG=warn,
  # so flip to "live" as soon as the first record arrives).
  while ($null -ne ($line = $rdUi.ReadLine())) {
    if (-not $line.StartsWith($us)) { continue }
    if (-not $script:ready) {
      $script:ready = $true
      $status.Text = "Live - speak Russian into the mic"
      $status.ForeColor = $accent
    }
    $parts = $line.Substring(1).Split("`t")
    $kind = $parts[0]
    $a = if ($parts.Count -gt 1) { $parts[1] } else { "" }
    $b = if ($parts.Count -gt 2) { $parts[2] } else { "" }
    switch ($kind) {
      "RU"    { $ruBox.Text = $a }
      "EN"    { $enBox.Text = $a }
      "FINAL" {
        $ruBox.Text = $a; $enBox.Text = $b
        if ($a -ne "") {
          $line2 = if ($b -ne "") { "RU: $a`r`nEN: $b`r`n" } else { "RU: $a  (no translation)`r`n" }
          $hist.AppendText($line2)
        }
      }
    }
  }
  if ($proc.HasExited -and -not $script:ready) {
    $status.Text = "Process exited (code $($proc.ExitCode)) - see $errFile"
    $status.ForeColor = [System.Drawing.Color]::IndianRed
  }
})
$timer.Start()

$form.Add_FormClosed({
  try { $timer.Stop() } catch {}
  try { Get-Process voice-translator -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
  try { if ($proc -and -not $proc.HasExited) { $proc.Kill() } } catch {}
  try { $rdUi.Dispose(); $rdErr.Dispose() } catch {}
})

[void]$form.ShowDialog()
