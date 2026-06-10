#requires -version 5
# Lightweight desktop UI for the voice translator.
# One window does everything: pick the output (speakers / virtual mic / mute),
# launch voice-translator.exe (--mode full, VOICE_UI=1), and show live RU -> EN.
# The output choice is passed to the engine via env vars (VOICE_TTS_*).

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()

# --- config ---
$proj    = "C:\Users\egorb\OneDrive\Documentos\GitHub\LocalLLMVoiceProject"
$uiFile  = Join-Path $env:TEMP "voice_ui_stream.txt"
$errFile = Join-Path $env:TEMP "voice_ui_err.txt"
$launcher = Join-Path $proj "_ui_launch.bat"
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
$form.Size = New-Object System.Drawing.Size(780,612)
$form.StartPosition = "CenterScreen"
$form.BackColor = $bg
$form.Font = New-Object System.Drawing.Font("Segoe UI",10)

# --- control bar: output selector + restart ---
$outTag = New-Object System.Windows.Forms.Label
$outTag.Text = "Output:"; $outTag.ForeColor = $fgDim; $outTag.AutoSize = $true
$outTag.Location = New-Object System.Drawing.Point(20,17)
$form.Controls.Add($outTag)

$combo = New-Object System.Windows.Forms.ComboBox
$combo.DropDownStyle = "DropDownList"
$combo.Location = New-Object System.Drawing.Point(78,13)
$combo.Size = New-Object System.Drawing.Size(360,28)
$combo.BackColor = $card; $combo.ForeColor = $fgRu
[void]$combo.Items.Add("Speakers  (you hear the translation)")
[void]$combo.Items.Add("Virtual mic -> Telegram  (CABLE Input)")
[void]$combo.Items.Add("No voice  (text only)")
$combo.SelectedIndex = 0
$form.Controls.Add($combo)

$btnStart = New-Object System.Windows.Forms.Button
$btnStart.Text = "Start / Restart"; $btnStart.Location = New-Object System.Drawing.Point(450,12)
$btnStart.Size = New-Object System.Drawing.Size(120,28)
$btnStart.ForeColor = $fgRu
$form.Controls.Add($btnStart)

# status
$status = New-Object System.Windows.Forms.Label
$status.Text = "Starting..."
$status.ForeColor = $fgDim
$status.AutoSize = $true
$status.Location = New-Object System.Drawing.Point(20,52)
$form.Controls.Add($status)

# RU card
$ruTag = New-Object System.Windows.Forms.Label
$ruTag.Text = "RU  (speech)"; $ruTag.ForeColor = $fgDim; $ruTag.AutoSize = $true
$ruTag.Location = New-Object System.Drawing.Point(22,82)
$form.Controls.Add($ruTag)

$ruBox = New-Object System.Windows.Forms.Label
$ruBox.BackColor = $card; $ruBox.ForeColor = $fgRu
$ruBox.Font = New-Object System.Drawing.Font("Segoe UI",17)
$ruBox.Location = New-Object System.Drawing.Point(20,104)
$ruBox.Size = New-Object System.Drawing.Size(720,90)
$ruBox.Padding = New-Object System.Windows.Forms.Padding(12)
$ruBox.TextAlign = "TopLeft"
$ruBox.Anchor = "Top,Left,Right"
$form.Controls.Add($ruBox)

# EN card
$enTag = New-Object System.Windows.Forms.Label
$enTag.Text = "EN  (translation)"; $enTag.ForeColor = $fgDim; $enTag.AutoSize = $true
$enTag.Location = New-Object System.Drawing.Point(22,206)
$form.Controls.Add($enTag)

$enBox = New-Object System.Windows.Forms.Label
$enBox.BackColor = $card; $enBox.ForeColor = $fgEn
$enBox.Font = New-Object System.Drawing.Font("Segoe UI",20,[System.Drawing.FontStyle]::Bold)
$enBox.Location = New-Object System.Drawing.Point(20,228)
$enBox.Size = New-Object System.Drawing.Size(720,100)
$enBox.Padding = New-Object System.Windows.Forms.Padding(12)
$enBox.TextAlign = "TopLeft"
$enBox.Anchor = "Top,Left,Right"
$form.Controls.Add($enBox)

# history
$histTag = New-Object System.Windows.Forms.Label
$histTag.Text = "History"; $histTag.ForeColor = $fgDim; $histTag.AutoSize = $true
$histTag.Location = New-Object System.Drawing.Point(22,340)
$form.Controls.Add($histTag)

$hist = New-Object System.Windows.Forms.TextBox
$hist.Multiline = $true; $hist.ReadOnly = $true; $hist.ScrollBars = "Vertical"
$hist.BackColor = $card; $hist.ForeColor = $fgRu
$hist.Font = New-Object System.Drawing.Font("Consolas",10)
$hist.Location = New-Object System.Drawing.Point(20,362)
$hist.Size = New-Object System.Drawing.Size(720,150)
$hist.Anchor = "Top,Bottom,Left,Right"
$form.Controls.Add($hist)

# buttons
$btnClear = New-Object System.Windows.Forms.Button
$btnClear.Text = "Clear"; $btnClear.Location = New-Object System.Drawing.Point(560,522)
$btnClear.Size = New-Object System.Drawing.Size(85,30)
$btnClear.Anchor = "Bottom,Right"
$btnClear.Add_Click({ $hist.Clear() })
$form.Controls.Add($btnClear)

$btnStop = New-Object System.Windows.Forms.Button
$btnStop.Text = "Quit"; $btnStop.Location = New-Object System.Drawing.Point(655,522)
$btnStop.Size = New-Object System.Drawing.Size(85,30)
$btnStop.Anchor = "Bottom,Right"
$btnStop.Add_Click({ $form.Close() })
$form.Controls.Add($btnStop)

# --- tail readers (recreated on each (re)start) ---
$us = [char]0x1f
$script:proc  = $null
$script:rdUi  = $null
$script:rdErr = $null
$script:fsUi  = $null
$script:fsErr = $null
$script:ready = $false

function Start-Pipeline {
  # stop any running instance
  try { Get-Process voice-translator -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
  if ($script:proc -and -not $script:proc.HasExited) { try { $script:proc.Kill() } catch {} }

  # output choice -> env (inherited by _ui_launch.bat -> the exe)
  $env:VOICE_TTS_SPEAK = "1"
  $env:VOICE_TTS_OUTPUT = ""
  switch ($combo.SelectedIndex) {
    0 { $env:VOICE_TTS_OUTPUT = "" }            # speakers
    1 { $env:VOICE_TTS_OUTPUT = "CABLE Input" } # virtual mic for Telegram
    2 { $env:VOICE_TTS_SPEAK = "0" }            # text only
  }

  # fresh output files + readers
  Set-Content -Path $uiFile -Value "" -Encoding UTF8
  Set-Content -Path $errFile -Value "" -Encoding UTF8
  if ($script:rdUi)  { try { $script:rdUi.Dispose() }  catch {} }
  if ($script:rdErr) { try { $script:rdErr.Dispose() } catch {} }
  $script:fsUi  = [System.IO.FileStream]::new($uiFile, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
  $script:rdUi  = [System.IO.StreamReader]::new($script:fsUi, [System.Text.Encoding]::UTF8)
  $script:fsErr = [System.IO.FileStream]::new($errFile,[System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
  $script:rdErr = [System.IO.StreamReader]::new($script:fsErr,[System.Text.Encoding]::UTF8)

  $script:ready = $false
  $status.Text = "Loading models (~15s), then speak Russian into the mic..."
  $status.ForeColor = $fgDim
  $ruBox.Text = ""; $enBox.Text = ""

  $script:proc = Start-Process -FilePath $launcher -WorkingDirectory $proj -WindowStyle Hidden -PassThru
}

$btnStart.Add_Click({ Start-Pipeline })

# --- tail timer ---
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 120
$timer.Add_Tick({
  if ($null -eq $script:rdUi) { return }
  while ($null -ne ($line = $script:rdUi.ReadLine())) {
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
  if ($script:proc -and $script:proc.HasExited -and -not $script:ready) {
    $status.Text = "Process exited (code $($script:proc.ExitCode)) - see $errFile"
    $status.ForeColor = [System.Drawing.Color]::IndianRed
  }
})
$timer.Start()

$form.Add_FormClosed({
  try { $timer.Stop() } catch {}
  try { Get-Process voice-translator -ErrorAction SilentlyContinue | Stop-Process -Force } catch {}
  try { if ($script:proc -and -not $script:proc.HasExited) { $script:proc.Kill() } } catch {}
  try { $script:rdUi.Dispose(); $script:rdErr.Dispose() } catch {}
})

# auto-start on open
Start-Pipeline

[void]$form.ShowDialog()
