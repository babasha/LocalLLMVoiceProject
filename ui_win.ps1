#requires -version 5
# Desktop UI for the local voice translator (Windows, WinForms - no extra deps).
#
#   Live     - "Me" (mic -> translation -> voice) and "Them" (system audio ->
#              subtitles) panels. Committed words (already voiced, final) are
#              bright, draft words (may still change) are dimmed.
#   Settings - mode, devices, voice output, LLM, VAD pause, UI language.
#   Models   - status of every model from config/models.json + downloads.
# Lives in the system tray; settings are kept in %APPDATA%\VoiceTranslator.
#
# The engine (voice-translator.exe) runs with VOICE_UI=1 and emits records
#   <US>KIND<TAB>chan<TAB>a<TAB>b  (READY / SRC / TR / FINAL / WARN)
# to stdout, which is tailed from a temp file.

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()

# ------------------------------------------------------------------ paths --
$proj = $PSScriptRoot
$cudaRoot = $(if ($env:CUDA_PATH) { $env:CUDA_PATH } else { "C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA\v13.3" })
$cudaBin = Join-Path $cudaRoot "bin"
$env:PATH = "$cudaBin\x64;$cudaBin;$proj\libs\sherpa-onnx-win;$env:PATH"

function Find-Exe {
  $cands = @()
  if ($env:VOICE_TRANSLATOR_EXE) { $cands += $env:VOICE_TRANSLATOR_EXE }
  if ($env:CARGO_TARGET_DIR) { $cands += (Join-Path $env:CARGO_TARGET_DIR "release\voice-translator.exe") }
  $cands += "C:\cargo-target\voice-translator\release\voice-translator.exe"
  $cands += (Join-Path $proj "target\release\voice-translator.exe")
  $cands += (Join-Path $proj "voice-translator.exe")
  foreach ($c in $cands) { if (Test-Path $c) { return $c } }
  return $null
}
$exe = Find-Exe

$uiFile  = Join-Path $env:TEMP "voice_ui_stream.txt"
$errFile = Join-Path $env:TEMP "voice_ui_err.txt"
$settingsDir  = Join-Path $env:APPDATA "VoiceTranslator"
$settingsFile = Join-Path $settingsDir "settings.json"
$manifest = (Get-Content (Join-Path $proj "config\models.json") -Raw -Encoding UTF8 | ConvertFrom-Json).models

# --------------------------------------------------------------- settings --
$S = [ordered]@{
  lang = "ru"; mode = "full"; mic = ""; loopback = ""; voice = "default";
  llm = "models/Qwen3.5-4B-Q4_K_M.gguf"; silence = 250; toTray = $true
}
if (Test-Path $settingsFile) {
  try {
    $saved = Get-Content $settingsFile -Raw -Encoding UTF8 | ConvertFrom-Json
    foreach ($p in $saved.PSObject.Properties) { $S[$p.Name] = $p.Value }
  } catch {}
}
function Save-Settings {
  try {
    New-Item -ItemType Directory -Force -Path $settingsDir | Out-Null
    ($S | ConvertTo-Json) | Set-Content -Path $settingsFile -Encoding UTF8
  } catch {}
}
function Resolve-ProjPath([string]$p) {
  if ([string]::IsNullOrWhiteSpace($p)) { return $p }
  if ([System.IO.Path]::IsPathRooted($p)) { return $p }
  return (Join-Path $proj $p)
}

# ------------------------------------------------------------------- i18n --
$L = @{
  ru = @{
    title = "Голосовой переводчик (локально, GPU)"; tabLive = "Перевод"; tabSettings = "Настройки"; tabModels = "Модели"
    start = "Старт"; stop = "Стоп"; clear = "Очистить"; quit = "Выход"; show = "Показать"
    me = "Я"; them = "Собеседник"; history = "История"
    stIdle = "Остановлено"; stLoading = "Загрузка моделей..."; stLive = "Работает"; stExited = "Процесс завершился (код {0}) — см. {1}"
    stNoExe = "Не найден voice-translator.exe — запустите build_win.bat"; stMissing = "Не хватает моделей: {0}. Скачайте их на вкладке «Модели»."
    mode = "Режим"; modeFull = "Я говорю → перевод голосом"; modeListen = "Слушаю собеседника → субтитры"; modeDuplex = "Разговор в обе стороны"
    mic = "Микрофон"; loop = "Звук собеседника (loopback)"; voice = "Озвучка перевода"; llm = "Модель перевода (LLM)"
    silence = "Пауза конца фразы, мс"; uiLang = "Язык интерфейса"; toTray = "Сворачивать в трей"
    defDev = "По умолчанию"; noVoice = "Без голоса (только текст)"; browse = "Обзор..."; apply = "Сохранить и перезапустить"
    hintCable = "Совет: чтобы собеседник в Telegram/Discord слышал перевод, выберите «CABLE Input» и укажите «CABLE Output» микрофоном в мессенджере."
    hintEcho = "Озвучка идёт туда же, откуда слушаем собеседника — его канал будет приглушаться на время озвучки. Лучше наушники или CABLE."
    colName = "Модель"; colPurpose = "Назначение"; colSize = "Размер"; colStatus = "Статус"
    ok = "Есть"; missing = "Нет"; notNeeded = "не нужна для режима"; download = "Скачать выбранные"; downloadMissing = "Скачать недостающие"
    dlDone = "Готово"; dlFail = "Ошибка загрузки: {0}"; dlExtract = "Распаковка {0}..."; dlGet = "Загрузка {0}: {1} МБ / {2} МБ"
    lagWarn = "Отставание"; ready = "Модели загружены — можно говорить"
  }
  en = @{
    title = "Voice Translator (local, GPU)"; tabLive = "Live"; tabSettings = "Settings"; tabModels = "Models"
    start = "Start"; stop = "Stop"; clear = "Clear"; quit = "Quit"; show = "Show"
    me = "Me"; them = "Them"; history = "History"
    stIdle = "Stopped"; stLoading = "Loading models..."; stLive = "Live"; stExited = "Process exited (code {0}) - see {1}"
    stNoExe = "voice-translator.exe not found - run build_win.bat"; stMissing = "Missing models: {0}. Get them on the Models tab."
    mode = "Mode"; modeFull = "I speak -> spoken translation"; modeListen = "Listen to the other party -> subtitles"; modeDuplex = "Two-way conversation"
    mic = "Microphone"; loop = "Other party audio (loopback)"; voice = "Spoken translation output"; llm = "Translation model (LLM)"
    silence = "End-of-phrase pause, ms"; uiLang = "UI language"; toTray = "Minimize to tray"
    defDev = "Default"; noVoice = "No voice (text only)"; browse = "Browse..."; apply = "Save and restart"
    hintCable = "Tip: to let a Telegram/Discord peer hear the translation, choose 'CABLE Input' and pick 'CABLE Output' as the mic in the messenger."
    hintEcho = "Voice goes to the device we capture the other party from - their channel is muted while speaking. Prefer headphones or CABLE."
    colName = "Model"; colPurpose = "Purpose"; colSize = "Size"; colStatus = "Status"
    ok = "Present"; missing = "Missing"; notNeeded = "not needed for mode"; download = "Download selected"; downloadMissing = "Download missing"
    dlDone = "Done"; dlFail = "Download failed: {0}"; dlExtract = "Extracting {0}..."; dlGet = "Downloading {0}: {1} MB / {2} MB"
    lagWarn = "Lag"; ready = "Models loaded - you can speak"
  }
}
function T([string]$k) { $d = $L[$S.lang]; if (-not $d) { $d = $L.en }; $v = $d[$k]; if ($v) { return $v } else { return $k } }

# ----------------------------------------------------------------- colors --
$bg      = [System.Drawing.Color]::FromArgb(24,26,32)
$card    = [System.Drawing.Color]::FromArgb(34,37,46)
$fgDim   = [System.Drawing.Color]::FromArgb(150,156,170)
$fgText  = [System.Drawing.Color]::FromArgb(232,234,240)
$fgTr    = [System.Drawing.Color]::FromArgb(120,200,255)
$fgDraft = [System.Drawing.Color]::FromArgb(110,170,130)
$accent  = [System.Drawing.Color]::FromArgb(90,180,120)
$warnC   = [System.Drawing.Color]::FromArgb(235,170,80)
$errC    = [System.Drawing.Color]::IndianRed

function New-Label($text, $x, $y, $color) {
  $l = New-Object System.Windows.Forms.Label
  $l.Text = $text; $l.ForeColor = $color; $l.AutoSize = $true
  $l.Location = New-Object System.Drawing.Point($x, $y)
  return $l
}

# ------------------------------------------------------------------- form --
$form = New-Object System.Windows.Forms.Form
$form.Size = New-Object System.Drawing.Size(860, 720)
$form.MinimumSize = New-Object System.Drawing.Size(640, 560)
$form.StartPosition = "CenterScreen"
$form.BackColor = $bg; $form.ForeColor = $fgText
$form.Font = New-Object System.Drawing.Font("Segoe UI", 10)
try { $form.Icon = [System.Drawing.Icon]::ExtractAssociatedIcon($exe) } catch {}

# top bar
$btnStart = New-Object System.Windows.Forms.Button
$btnStart.Location = New-Object System.Drawing.Point(16, 12); $btnStart.Size = New-Object System.Drawing.Size(110, 30)
$btnStop = New-Object System.Windows.Forms.Button
$btnStop.Location = New-Object System.Drawing.Point(134, 12); $btnStop.Size = New-Object System.Drawing.Size(90, 30)
$status = New-Label "" 236 18 $fgDim
$form.Controls.AddRange(@($btnStart, $btnStop, $status))

$tabs = New-Object System.Windows.Forms.TabControl
$tabs.Location = New-Object System.Drawing.Point(12, 52)
$tabs.Size = New-Object System.Drawing.Size(820, 610)
$tabs.Anchor = "Top,Bottom,Left,Right"
$form.Controls.Add($tabs)
$tabLive = New-Object System.Windows.Forms.TabPage; $tabLive.BackColor = $bg
$tabSet  = New-Object System.Windows.Forms.TabPage; $tabSet.BackColor = $bg
$tabMod  = New-Object System.Windows.Forms.TabPage; $tabMod.BackColor = $bg
$tabs.TabPages.AddRange(@($tabLive, $tabSet, $tabMod))

# ------------------------------------------------------------- live panels --
function New-ChannelPanel($y) {
  $p = @{}
  $p.title = New-Label "" 10 $y $fgDim
  $p.warn  = New-Label "" 300 $y $warnC
  $p.src = New-Object System.Windows.Forms.Label
  $p.src.BackColor = $card; $p.src.ForeColor = $fgText
  $p.src.Font = New-Object System.Drawing.Font("Segoe UI", 14)
  $p.src.Location = New-Object System.Drawing.Point(8, ($y + 22)); $p.src.Size = New-Object System.Drawing.Size(790, 62)
  $p.src.Padding = New-Object System.Windows.Forms.Padding(8); $p.src.Anchor = "Top,Left,Right"
  $p.tr = New-Object System.Windows.Forms.RichTextBox
  $p.tr.BackColor = $card; $p.tr.ForeColor = $fgTr; $p.tr.ReadOnly = $true; $p.tr.BorderStyle = "None"
  $p.tr.Font = New-Object System.Drawing.Font("Segoe UI", 17, [System.Drawing.FontStyle]::Bold)
  $p.tr.Location = New-Object System.Drawing.Point(8, ($y + 88)); $p.tr.Size = New-Object System.Drawing.Size(790, 74)
  $p.tr.ScrollBars = "None"; $p.tr.Anchor = "Top,Left,Right"
  $tabLive.Controls.AddRange(@($p.title, $p.warn, $p.src, $p.tr))
  return $p
}
$panel = @{ me = (New-ChannelPanel 6); them = (New-ChannelPanel 178) }

$histTag = New-Label "" 10 352 $fgDim
$hist = New-Object System.Windows.Forms.RichTextBox
$hist.ReadOnly = $true; $hist.BackColor = $card; $hist.ForeColor = $fgText; $hist.BorderStyle = "None"
$hist.Font = New-Object System.Drawing.Font("Segoe UI", 10)
$hist.Location = New-Object System.Drawing.Point(8, 374); $hist.Size = New-Object System.Drawing.Size(790, 160)
$hist.Anchor = "Top,Bottom,Left,Right"
$btnClear = New-Object System.Windows.Forms.Button
$btnClear.Location = New-Object System.Drawing.Point(708, 542); $btnClear.Size = New-Object System.Drawing.Size(90, 28)
$btnClear.Anchor = "Bottom,Right"
$btnClear.Add_Click({ $hist.Clear() })
$tabLive.Controls.AddRange(@($histTag, $hist, $btnClear))

function Set-Translation($p, [string]$committed, [string]$draft) {
  $rt = $p.tr
  $rt.SuspendLayout()
  $rt.Clear()
  $rt.SelectionColor = $fgTr
  $rt.AppendText($committed)
  if ($draft) {
    $rt.SelectionColor = $fgDraft
    if ($committed) { $rt.AppendText(" ") }
    $rt.AppendText($draft)
  }
  # Long text: keep the newest words in view.
  $rt.SelectionStart = $rt.TextLength; $rt.ScrollToCaret()
  $rt.ResumeLayout()
}

# Live source text can run long (continuous speech): show only its tail.
function Tail-Text([string]$s, [int]$max = 150) {
  if ($s.Length -le $max) { return $s }
  $cut = $s.Substring($s.Length - $max)
  $sp = $cut.IndexOf(" ")
  if ($sp -gt 0 -and $sp -lt 30) { $cut = $cut.Substring($sp + 1) }
  return [char]0x2026 + " " + $cut
}

# Lay the visible channel panels out from the top and stretch them to the tab.
function Layout-Live {
  $w = [Math]::Max(300, $tabLive.ClientSize.Width - 16)
  $y = 6
  foreach ($k in @("me", "them")) {
    $p = $panel[$k]
    if (-not $p.shown) { continue }  # (.Visible reads false while the form is hidden)
    $p.title.Top = $y; $p.warn.Top = $y; $p.warn.Left = [Math]::Min(300, $w - 200)
    $p.src.Top = $y + 22; $p.src.Width = $w
    $p.tr.Top = $y + 88; $p.tr.Width = $w
    $y += 172
  }
  $histTag.Top = $y; $hist.Top = $y + 22; $hist.Width = $w
  $hist.Height = [Math]::Max(60, $btnClear.Top - $hist.Top - 8)
}
$tabLive.Add_Resize({ Layout-Live })

function Add-History([string]$chan, [string]$src, [string]$tr) {
  if (-not $src) { return }
  $who = if ($chan -eq "them") { T "them" } else { T "me" }
  $hist.SelectionColor = $fgDim;  $hist.AppendText("$who  ")
  $hist.SelectionColor = $fgText; $hist.AppendText("$src`r`n")
  # Same text = it was already in the target language (no translation needed).
  if ($tr -and $tr -ne $src) { $hist.SelectionColor = $fgTr; $hist.AppendText("      $tr`r`n") }
  $hist.SelectionStart = $hist.TextLength; $hist.ScrollToCaret()
}

# ------------------------------------------------------------ settings tab --
function New-Combo($x, $y, $w) {
  $c = New-Object System.Windows.Forms.ComboBox
  $c.DropDownStyle = "DropDownList"; $c.BackColor = $card; $c.ForeColor = $fgText
  $c.Location = New-Object System.Drawing.Point($x, $y); $c.Size = New-Object System.Drawing.Size($w, 28)
  return $c
}
$lblMode = New-Label "" 16 20 $fgDim;    $cbMode = New-Combo 280 16 500
$lblMic  = New-Label "" 16 60 $fgDim;    $cbMic  = New-Combo 280 56 500
$lblLoop = New-Label "" 16 100 $fgDim;   $cbLoop = New-Combo 280 96 500
$lblVoice = New-Label "" 16 140 $fgDim;  $cbVoice = New-Combo 280 136 500
$lblLlm  = New-Label "" 16 180 $fgDim;   $cbLlm  = New-Combo 280 176 400
$btnBrowse = New-Object System.Windows.Forms.Button
$btnBrowse.Location = New-Object System.Drawing.Point(688, 175); $btnBrowse.Size = New-Object System.Drawing.Size(92, 28)
$lblSil  = New-Label "" 16 220 $fgDim
$numSil = New-Object System.Windows.Forms.NumericUpDown
$numSil.Minimum = 100; $numSil.Maximum = 2000; $numSil.Increment = 50
$numSil.Location = New-Object System.Drawing.Point(280, 216); $numSil.Size = New-Object System.Drawing.Size(100, 28)
$numSil.BackColor = $card; $numSil.ForeColor = $fgText
$lblLang = New-Label "" 16 260 $fgDim;   $cbLang = New-Combo 280 256 160
[void]$cbLang.Items.AddRange(@("Русский", "English"))
$chkTray = New-Object System.Windows.Forms.CheckBox
$chkTray.Location = New-Object System.Drawing.Point(280, 296); $chkTray.AutoSize = $true; $chkTray.ForeColor = $fgText
$lblHint = New-Label "" 16 340 $fgDim
$lblHint.AutoSize = $false; $lblHint.Size = New-Object System.Drawing.Size(770, 70)
$btnApply = New-Object System.Windows.Forms.Button
$btnApply.Location = New-Object System.Drawing.Point(280, 420); $btnApply.Size = New-Object System.Drawing.Size(220, 32)
$tabSet.Controls.AddRange(@($lblMode, $cbMode, $lblMic, $cbMic, $lblLoop, $cbLoop, $lblVoice, $cbVoice,
  $lblLlm, $cbLlm, $btnBrowse, $lblSil, $numSil, $lblLang, $cbLang, $chkTray, $lblHint, $btnApply))

$modeKeys = @("full", "listen", "duplex")
$script:inDev = @(); $script:outDev = @(); $script:llmPaths = @()

function Load-Devices {
  $script:inDev = @(); $script:outDev = @()
  if (-not $exe) { return }
  try {
    $lines = & $exe --list-devices --tsv 2>$null
    foreach ($ln in $lines) {
      $parts = "$ln".Split("`t")
      if ($parts.Count -lt 2) { continue }
      if ($parts[0].StartsWith("IN")) { $script:inDev += $parts[1] }
      elseif ($parts[0].StartsWith("OUT")) { $script:outDev += $parts[1] }
    }
  } catch {}
}

function Load-Llms {
  $found = New-Object System.Collections.Generic.List[string]
  $dirs = @((Join-Path $proj "models"), (Join-Path $env:USERPROFILE ".lmstudio\models"))
  foreach ($d in $dirs) {
    if (Test-Path $d) {
      Get-ChildItem -Path $d -Recurse -Filter *.gguf -ErrorAction SilentlyContinue |
        Where-Object { $_.Name -notlike "mmproj*" } |
        ForEach-Object {
          $full = $_.FullName
          if ($full.StartsWith($proj)) { $full = $full.Substring($proj.Length + 1).Replace("\", "/") }
          if (-not $found.Contains($full)) { $found.Add($full) }
        }
    }
  }
  if ($S.llm -and -not $found.Contains($S.llm)) { $found.Insert(0, $S.llm) }
  $script:llmPaths = @($found)
}

function Fill-Settings {
  $cbMode.Items.Clear(); [void]$cbMode.Items.AddRange(@((T "modeFull"), (T "modeListen"), (T "modeDuplex")))
  $cbMode.SelectedIndex = [Math]::Max(0, [Array]::IndexOf($modeKeys, $S.mode))

  $cbMic.Items.Clear(); [void]$cbMic.Items.Add((T "defDev"))
  foreach ($d in $script:inDev) { [void]$cbMic.Items.Add($d) }
  $i = [Array]::IndexOf($script:inDev, $S.mic); $cbMic.SelectedIndex = [Math]::Max(0, $i + 1)

  $cbLoop.Items.Clear(); [void]$cbLoop.Items.Add((T "defDev"))
  foreach ($d in $script:outDev) { [void]$cbLoop.Items.Add($d) }
  $i = [Array]::IndexOf($script:outDev, $S.loopback); $cbLoop.SelectedIndex = [Math]::Max(0, $i + 1)

  $cbVoice.Items.Clear(); [void]$cbVoice.Items.Add((T "defDev")); [void]$cbVoice.Items.Add((T "noVoice"))
  foreach ($d in $script:outDev) { [void]$cbVoice.Items.Add($d) }
  if ($S.voice -eq "none") { $cbVoice.SelectedIndex = 1 }
  else {
    $i = [Array]::IndexOf($script:outDev, $S.voice)
    $cbVoice.SelectedIndex = $(if ($i -ge 0) { $i + 2 } else { 0 })
  }

  $cbLlm.Items.Clear(); foreach ($p in $script:llmPaths) { [void]$cbLlm.Items.Add($p) }
  $i = [Array]::IndexOf($script:llmPaths, $S.llm); if ($cbLlm.Items.Count -gt 0) { $cbLlm.SelectedIndex = [Math]::Max(0, $i) }

  $numSil.Value = [Math]::Min(2000, [Math]::Max(100, [int]$S.silence))
  $cbLang.SelectedIndex = $(if ($S.lang -eq "en") { 1 } else { 0 })
  $chkTray.Checked = [bool]$S.toTray
  Update-Hint
}

function Read-Settings {
  $S.mode = $modeKeys[[Math]::Max(0, $cbMode.SelectedIndex)]
  $S.mic = $(if ($cbMic.SelectedIndex -gt 0) { [string]$cbMic.SelectedItem } else { "" })
  $S.loopback = $(if ($cbLoop.SelectedIndex -gt 0) { [string]$cbLoop.SelectedItem } else { "" })
  $S.voice = $(switch ($cbVoice.SelectedIndex) { 0 { "default" } 1 { "none" } default { [string]$cbVoice.SelectedItem } })
  if ($cbLlm.SelectedItem) { $S.llm = [string]$cbLlm.SelectedItem }
  $S.silence = [int]$numSil.Value
  $S.lang = $(if ($cbLang.SelectedIndex -eq 1) { "en" } else { "ru" })
  $S.toTray = $chkTray.Checked
}

function Update-Hint {
  $mode = $modeKeys[[Math]::Max(0, $cbMode.SelectedIndex)]
  $voiceIdx = $cbVoice.SelectedIndex
  $voiceDev = $(if ($voiceIdx -ge 2) { [string]$cbVoice.SelectedItem } else { "" })
  $loopDev = $(if ($cbLoop.SelectedIndex -gt 0) { [string]$cbLoop.SelectedItem } else { "" })
  if ($mode -eq "duplex" -and $voiceIdx -ne 1 -and $voiceDev -eq $loopDev) { $lblHint.Text = T "hintEcho"; $lblHint.ForeColor = $warnC }
  else { $lblHint.Text = T "hintCable"; $lblHint.ForeColor = $fgDim }
}
$cbMode.Add_SelectedIndexChanged({ Update-Hint })
$cbVoice.Add_SelectedIndexChanged({ Update-Hint })
$cbLoop.Add_SelectedIndexChanged({ Update-Hint })

$btnBrowse.Add_Click({
  $dlg = New-Object System.Windows.Forms.OpenFileDialog
  $dlg.Filter = "GGUF (*.gguf)|*.gguf"
  if ($dlg.ShowDialog() -eq "OK") {
    if (-not $script:llmPaths.Contains($dlg.FileName)) { $script:llmPaths += $dlg.FileName; [void]$cbLlm.Items.Add($dlg.FileName) }
    $cbLlm.SelectedItem = $dlg.FileName
  }
})

$btnApply.Add_Click({
  $oldLang = $S.lang
  Read-Settings; Save-Settings
  if ($oldLang -ne $S.lang) { Apply-Language }
  Refresh-Models
  $tabs.SelectedTab = $tabLive
  Start-Pipeline
})

# -------------------------------------------------------------- models tab --
$lv = New-Object System.Windows.Forms.ListView
$lv.View = "Details"; $lv.FullRowSelect = $true; $lv.CheckBoxes = $true
$lv.BackColor = $card; $lv.ForeColor = $fgText; $lv.BorderStyle = "None"
$lv.Location = New-Object System.Drawing.Point(8, 10); $lv.Size = New-Object System.Drawing.Size(790, 300)
$lv.Anchor = "Top,Left,Right"
[void]$lv.Columns.Add("", 240); [void]$lv.Columns.Add("", 280); [void]$lv.Columns.Add("", 80); [void]$lv.Columns.Add("", 170)
$btnDlSel = New-Object System.Windows.Forms.Button
$btnDlSel.Location = New-Object System.Drawing.Point(8, 320); $btnDlSel.Size = New-Object System.Drawing.Size(200, 30)
$btnDlMiss = New-Object System.Windows.Forms.Button
$btnDlMiss.Location = New-Object System.Drawing.Point(216, 320); $btnDlMiss.Size = New-Object System.Drawing.Size(200, 30)
$prog = New-Object System.Windows.Forms.ProgressBar
$prog.Location = New-Object System.Drawing.Point(8, 362); $prog.Size = New-Object System.Drawing.Size(790, 20); $prog.Anchor = "Top,Left,Right"
$dlStatus = New-Label "" 8 390 $fgDim
$tabMod.Controls.AddRange(@($lv, $btnDlSel, $btnDlMiss, $prog, $dlStatus))

function Model-Present($m) {
  if ($m.llm) { return (Test-Path (Resolve-ProjPath $S.llm)) -or (Test-Path (Resolve-ProjPath $m.check)) }
  return Test-Path (Resolve-ProjPath $m.check)
}
function Model-Needed($m, $mode) { return @($m.needed) -contains $mode }

function Refresh-Models {
  $lv.Items.Clear()
  foreach ($m in $manifest) {
    $purpose = $m.purpose.($S.lang); if (-not $purpose) { $purpose = $m.purpose.en }
    $it = New-Object System.Windows.Forms.ListViewItem($m.name)
    [void]$it.SubItems.Add($purpose)
    [void]$it.SubItems.Add(("{0} MB" -f $m.size_mb))
    $present = Model-Present $m
    $st = $(if ($present) { T "ok" } else { T "missing" })
    if (-not (Model-Needed $m $S.mode)) { $st = "$st ($(T 'notNeeded'))" }
    [void]$it.SubItems.Add($st)
    $it.ForeColor = $(if ($present) { $accent } elseif (Model-Needed $m $S.mode) { $warnC } else { $fgDim })
    $it.Tag = $m
    [void]$lv.Items.Add($it)
  }
}

function Missing-For-Mode {
  $miss = @()
  foreach ($m in $manifest) { if ((Model-Needed $m $S.mode) -and -not (Model-Present $m)) { $miss += $m } }
  return $miss
}

# Download queue: list of jobs { model, url, dest, archive }
$script:dlQueue = New-Object System.Collections.Queue
$script:dlBusy = $false
$script:tarProc = $null
$script:dlJob = $null
$wc = New-Object System.Net.WebClient
$wc.Headers.Add("User-Agent", "voice-translator-ui")

function Queue-Model($m) {
  if ($m.archive) {
    $tmp = Join-Path $env:TEMP ([System.IO.Path]::GetFileName($m.archive))
    $script:dlQueue.Enqueue(@{ name = $m.name; url = $m.archive; dest = $tmp; archive = $true })
  } else {
    foreach ($f in $m.files) {
      $dest = Resolve-ProjPath $f.path
      if (-not (Test-Path $dest)) { $script:dlQueue.Enqueue(@{ name = $m.name; url = $f.url; dest = $dest; archive = $false }) }
    }
  }
}

function Next-Download {
  if ($script:dlQueue.Count -eq 0) {
    $script:dlBusy = $false; $prog.Value = 0
    $dlStatus.Text = T "dlDone"; $dlStatus.ForeColor = $accent
    $btnDlSel.Enabled = $true; $btnDlMiss.Enabled = $true
    Refresh-Models
    return
  }
  $script:dlBusy = $true; $btnDlSel.Enabled = $false; $btnDlMiss.Enabled = $false
  $job = $script:dlQueue.Dequeue(); $script:dlJob = $job
  New-Item -ItemType Directory -Force -Path ([System.IO.Path]::GetDirectoryName($job.dest)) | Out-Null
  $part = "$($job.dest).part"
  $wc.DownloadFileAsync([Uri]$job.url, $part, $job)
}

$wc.Add_DownloadProgressChanged({
  param($s, $e)
  $prog.Value = [Math]::Min(100, [Math]::Max(0, $e.ProgressPercentage))
  $dlStatus.ForeColor = $fgDim
  $dlStatus.Text = [string]::Format((T "dlGet"), $script:dlJob.name, [int]($e.BytesReceived / 1MB), [int]($e.TotalBytesToReceive / 1MB))
})
$wc.Add_DownloadFileCompleted({
  param($s, $e)
  $job = $e.UserState
  $part = "$($job.dest).part"
  if ($e.Error -or $e.Cancelled) {
    Remove-Item -LiteralPath $part -ErrorAction SilentlyContinue
    $script:dlQueue.Clear()
    $dlStatus.Text = [string]::Format((T "dlFail"), $(if ($e.Error) { $e.Error.Message } else { "cancelled" }))
    $dlStatus.ForeColor = $errC
    $script:dlBusy = $false; $btnDlSel.Enabled = $true; $btnDlMiss.Enabled = $true
    return
  }
  Move-Item -LiteralPath $part -Destination $job.dest -Force
  if ($job.archive) {
    $dlStatus.Text = [string]::Format((T "dlExtract"), $job.name)
    $tar = Join-Path $env:SystemRoot "System32\tar.exe"
    $script:tarProc = Start-Process -FilePath $tar -ArgumentList @("-xjf", "`"$($job.dest)`"", "-C", "`"$(Join-Path $proj 'models')`"") -WindowStyle Hidden -PassThru
  } else {
    Next-Download
  }
})

$btnDlSel.Add_Click({
  foreach ($it in $lv.CheckedItems) { if (-not (Model-Present $it.Tag)) { Queue-Model $it.Tag } }
  if (-not $script:dlBusy) { Next-Download }
})
$btnDlMiss.Add_Click({
  foreach ($m in (Missing-For-Mode)) { Queue-Model $m }
  if (-not $script:dlBusy) { Next-Download }
})

# ----------------------------------------------------------------- engine --
$us = [char]0x1f
$script:proc = $null; $script:rdUi = $null; $script:fsUi = $null
$script:ready = $false; $script:running = $false

function Set-Status($text, $color) { $status.Text = $text; $status.ForeColor = $color; $tray.Text = ("Voice Translator - " + $text).Substring(0, [Math]::Min(63, 19 + $text.Length)) }

function Stop-Pipeline {
  $script:running = $false
  if ($script:proc -and -not $script:proc.HasExited) { try { $script:proc.Kill() } catch {} }
  $script:proc = $null
  Set-Status (T "stIdle") $fgDim
}

function Update-PanelVisibility {
  $showMe = $S.mode -ne "listen"; $showThem = $S.mode -ne "full"
  $panel.me.shown = $showMe; $panel.them.shown = $showThem
  foreach ($c in @($panel.me.title, $panel.me.warn, $panel.me.src, $panel.me.tr)) { $c.Visible = $showMe }
  foreach ($c in @($panel.them.title, $panel.them.warn, $panel.them.src, $panel.them.tr)) { $c.Visible = $showThem }
  Layout-Live
}

function Start-Pipeline {
  Stop-Pipeline
  Update-PanelVisibility
  if (-not $exe) { Set-Status (T "stNoExe") $errC; return }
  $miss = Missing-For-Mode
  if ($miss.Count -gt 0) {
    Set-Status ([string]::Format((T "stMissing"), (($miss | ForEach-Object { $_.name }) -join ", "))) $warnC
    Refresh-Models; $tabs.SelectedTab = $tabMod
    return
  }

  Set-Content -Path $uiFile -Value "" -Encoding UTF8
  if ($script:rdUi) { try { $script:rdUi.Dispose() } catch {} }
  $script:fsUi = [System.IO.FileStream]::new($uiFile, [System.IO.FileMode]::Open, [System.IO.FileAccess]::Read, [System.IO.FileShare]::ReadWrite)
  $script:rdUi = [System.IO.StreamReader]::new($script:fsUi, [System.Text.Encoding]::UTF8)

  $argv = @("--mode", $S.mode, "--silence-ms", "$($S.silence)", "--llm-model", "`"$(Resolve-ProjPath $S.llm)`"")
  $argv += @("--input-device", "`"$($S.mic)`"", "--loopback-device", "`"$($S.loopback)`"")
  if ($S.voice -eq "none") { $argv += @("--speak", "false") }
  else {
    $dev = $(if ($S.voice -eq "default") { "" } else { $S.voice })
    $argv += @("--speak", "true", "--tts-output", "`"$dev`"")
  }
  $env:VOICE_UI = "1"; $env:RUST_LOG = "warn"
  Remove-Item Env:VOICE_TTS_OUTPUT -ErrorAction SilentlyContinue
  Remove-Item Env:VOICE_TTS_SPEAK -ErrorAction SilentlyContinue

  foreach ($p in $panel.Values) { $p.src.Text = ""; Set-Translation $p "" ""; $p.warn.Text = "" }
  $script:ready = $false; $script:running = $true
  Set-Status (T "stLoading") $fgDim
  $script:proc = Start-Process -FilePath $exe -ArgumentList $argv -WorkingDirectory $proj -WindowStyle Hidden -PassThru `
    -RedirectStandardOutput $uiFile -RedirectStandardError $errFile
}

$btnStart.Add_Click({ Start-Pipeline })
$btnStop.Add_Click({ Stop-Pipeline })

# ------------------------------------------------------------------- tray --
$tray = New-Object System.Windows.Forms.NotifyIcon
$tray.Icon = $(if ($form.Icon) { $form.Icon } else { [System.Drawing.SystemIcons]::Application })
$tray.Visible = $true
$menu = New-Object System.Windows.Forms.ContextMenuStrip
$miShow = $menu.Items.Add(""); $miStart = $menu.Items.Add(""); $miStop = $menu.Items.Add("")
[void]$menu.Items.Add("-"); $miQuit = $menu.Items.Add("")
$tray.ContextMenuStrip = $menu
function Show-Main { $form.Show(); $form.WindowState = "Normal"; $form.Activate() }
$miShow.Add_Click({ Show-Main })
$miStart.Add_Click({ Start-Pipeline })
$miStop.Add_Click({ Stop-Pipeline })
$script:quitting = $false
$miQuit.Add_Click({ $script:quitting = $true; $form.Close() })
$tray.Add_DoubleClick({ Show-Main })
$form.Add_Resize({ if ($form.WindowState -eq "Minimized" -and $S.toTray) { $form.Hide() } })

# --------------------------------------------------------------- language --
function Apply-Language {
  $form.Text = T "title"
  $tabLive.Text = T "tabLive"; $tabSet.Text = T "tabSettings"; $tabMod.Text = T "tabModels"
  $btnStart.Text = T "start"; $btnStop.Text = T "stop"; $btnClear.Text = T "clear"
  $panel.me.title.Text = "$(T 'me')   RU -> EN"; $panel.them.title.Text = "$(T 'them')   EN -> RU"
  $histTag.Text = T "history"
  $lblMode.Text = T "mode"; $lblMic.Text = T "mic"; $lblLoop.Text = T "loop"; $lblVoice.Text = T "voice"
  $lblLlm.Text = T "llm"; $lblSil.Text = T "silence"; $lblLang.Text = T "uiLang"; $chkTray.Text = T "toTray"
  $btnBrowse.Text = T "browse"; $btnApply.Text = T "apply"
  $lv.Columns[0].Text = T "colName"; $lv.Columns[1].Text = T "colPurpose"; $lv.Columns[2].Text = T "colSize"; $lv.Columns[3].Text = T "colStatus"
  $btnDlSel.Text = T "download"; $btnDlMiss.Text = T "downloadMissing"
  $miShow.Text = T "show"; $miStart.Text = T "start"; $miStop.Text = T "stop"; $miQuit.Text = T "quit"
  Fill-Settings
  if (-not $script:running) { Set-Status (T "stIdle") $fgDim }
}

# ------------------------------------------------------------- tail timer --
$timer = New-Object System.Windows.Forms.Timer
$timer.Interval = 100
$timer.Add_Tick({
  # archive extraction finished?
  if ($script:tarProc -and $script:tarProc.HasExited) {
    $code = $script:tarProc.ExitCode; $script:tarProc = $null
    Remove-Item -LiteralPath $script:dlJob.dest -ErrorAction SilentlyContinue
    if ($code -ne 0) { $dlStatus.Text = [string]::Format((T "dlFail"), "tar exit $code"); $dlStatus.ForeColor = $errC; $script:dlQueue.Clear() }
    Next-Download
  }

  if ($null -eq $script:rdUi) { return }
  while ($null -ne ($line = $script:rdUi.ReadLine())) {
    if (-not $line.StartsWith($us)) { continue }
    $parts = $line.Substring(1).Split("`t")
    $kind = $parts[0]
    $chan = $(if ($parts.Count -gt 1) { $parts[1] } else { "" })
    $a = $(if ($parts.Count -gt 2) { $parts[2] } else { "" })
    $b = $(if ($parts.Count -gt 3) { $parts[3] } else { "" })
    $p = $panel[$chan]
    switch ($kind) {
      "READY" {
        $script:ready = $true
        Set-Status (T "stLive") $accent
        if (-not $form.Visible) { $tray.ShowBalloonTip(2000, "Voice Translator", (T "ready"), "Info") }
      }
      "SRC"   { if ($p) { $p.src.Text = Tail-Text $a } }
      "TR"    { if ($p) { Set-Translation $p $a $b } }
      "FINAL" { if ($p) { $p.src.Text = Tail-Text $a; Set-Translation $p $b "" }; Add-History $chan $a $b }
      "WARN"  { if ($p) { $p.warn.Text = $(if ($a) { "$(T 'lagWarn'): $a" } else { "" }) } }
    }
  }
  if ($script:running -and $script:proc -and $script:proc.HasExited) {
    $script:running = $false
    Set-Status ([string]::Format((T "stExited"), $script:proc.ExitCode, $errFile)) $errC
  }
})

$form.Add_FormClosing({
  param($s, $e)
  if (-not $script:quitting -and $S.toTray -and $e.CloseReason -eq "UserClosing") {
    $e.Cancel = $true; $form.Hide(); return
  }
})
$form.Add_FormClosed({
  try { $timer.Stop() } catch {}
  try { $wc.CancelAsync() } catch {}
  Stop-Pipeline
  try { $script:rdUi.Dispose() } catch {}
  $tray.Visible = $false; $tray.Dispose()
  Read-Settings; Save-Settings
})

# ------------------------------------------------------------------ start --
Load-Devices
Load-Llms
Apply-Language
Refresh-Models
Update-PanelVisibility
$timer.Start()
Start-Pipeline
[System.Windows.Forms.Application]::Run($form)
