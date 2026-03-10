const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

// --- State ---
let state = {
  isRunning: false,
  mode: 'passthrough',
};

// --- DOM ---
const statusBadge = document.getElementById('statusBadge');
const startBtn = document.getElementById('startBtn');
const startText = document.getElementById('startText');
const sourceLang = document.getElementById('sourceLang');
const targetLang = document.getElementById('targetLang');
const swapLangs = document.getElementById('swapLangs');
const inputDevice = document.getElementById('inputDevice');
const outputDevice = document.getElementById('outputDevice');
const levelBar = document.getElementById('levelBar');
const levelLabel = document.getElementById('levelLabel');
const testInput = document.getElementById('testInput');
const testBtn = document.getElementById('testBtn');
const testResult = document.getElementById('testResult');
const modelPath = document.getElementById('modelPath');
const gpuLayers = document.getElementById('gpuLayers');
const modeBtns = document.querySelectorAll('.mode-btn');

// --- Init ---
async function init() {
  await refreshStatus();
  await loadDevices();
  await invoke('start_monitoring');
  setupMicLevelListener();
}

async function refreshStatus() {
  try {
    const status = await invoke('get_status');
    state.isRunning = status.is_running;
    state.mode = status.mode;

    sourceLang.value = status.source_lang;
    targetLang.value = status.target_lang;
    modelPath.textContent = status.model_path;
    gpuLayers.textContent = status.gpu_layers;

    updateRunningUI();
    updateModeUI();
  } catch (e) {
    console.error('Failed to get status:', e);
  }
}

function updateRunningUI() {
  if (state.isRunning) {
    statusBadge.textContent = 'Running';
    statusBadge.classList.add('running');
    startText.textContent = 'Stop Translation';
    startBtn.classList.add('running');
  } else {
    statusBadge.textContent = 'Stopped';
    statusBadge.classList.remove('running');
    startText.textContent = 'Start Translation';
    startBtn.classList.remove('running');
  }
}

function updateModeUI() {
  modeBtns.forEach(btn => {
    btn.classList.toggle('active', btn.dataset.mode === state.mode);
  });
}

async function loadDevices() {
  try {
    const devices = await invoke('list_audio_devices');

    // Populate input devices
    const inputs = devices.filter(d => d.is_input);
    const outputs = devices.filter(d => !d.is_input);

    inputDevice.innerHTML = '';
    inputs.forEach(dev => {
      const opt = document.createElement('option');
      opt.value = dev.name;
      opt.textContent = dev.name + (dev.is_default ? ' (Default)' : '');
      opt.selected = dev.is_selected;
      inputDevice.appendChild(opt);
    });

    outputDevice.innerHTML = '';
    outputs.forEach(dev => {
      const opt = document.createElement('option');
      opt.value = dev.name;
      opt.textContent = dev.name + (dev.is_default ? ' (Default)' : '');
      opt.selected = dev.is_selected;
      outputDevice.appendChild(opt);
    });
  } catch (e) {
    console.error('Failed to load devices:', e);
  }
}

// --- Mic Level Meter with smoothing ---
let smoothLevel = 0;
let peakHold = 0;
let peakDecay = 0;

function setupMicLevelListener() {
  // Listen for events from backend
  listen('mic-level', (event) => {
    const level = event.payload;
    updateLevelMeter(level);
  });

  // Also poll as fallback every 50ms
  setInterval(async () => {
    try {
      const level = await invoke('get_mic_level');
      if (level > 0) updateLevelMeter(level);
    } catch (e) {}
  }, 50);

  // Decay animation
  setInterval(() => {
    if (smoothLevel > 0.001) {
      smoothLevel *= 0.85;
      renderLevel(smoothLevel);
    }
  }, 30);
}

function updateLevelMeter(level) {
  // Fast attack, slow release
  if (level > smoothLevel) {
    smoothLevel = level;
  } else {
    smoothLevel = smoothLevel * 0.7 + level * 0.3;
  }
  renderLevel(smoothLevel);
}

function renderLevel(level) {
  const pct = Math.min(level * 100, 100);
  levelBar.style.width = pct + '%';

  // dB display
  const db = level > 0.001 ? Math.max(-60, 20 * Math.log10(level)) : -60;
  levelLabel.textContent = db.toFixed(0) + ' dB';
}

// --- Events ---

startBtn.addEventListener('click', async () => {
  try {
    const status = await invoke('toggle_pipeline');
    state.isRunning = status.is_running;
    updateRunningUI();
  } catch (e) {
    console.error('Toggle failed:', e);
  }
});

modeBtns.forEach(btn => {
  btn.addEventListener('click', async () => {
    const mode = btn.dataset.mode;
    try {
      await invoke('set_mode', { mode });
      state.mode = mode;
      updateModeUI();
    } catch (e) {
      console.error('Set mode failed:', e);
    }
  });
});

inputDevice.addEventListener('change', async () => {
  try {
    await invoke('select_input_device', { name: inputDevice.value });
  } catch (e) {
    console.error('Select input device failed:', e);
  }
});

outputDevice.addEventListener('change', async () => {
  try {
    await invoke('select_output_device', { name: outputDevice.value });
  } catch (e) {
    console.error('Select output device failed:', e);
  }
});

sourceLang.addEventListener('change', async () => {
  await invoke('set_languages', {
    source: sourceLang.value,
    target: targetLang.value,
  });
});

targetLang.addEventListener('change', async () => {
  await invoke('set_languages', {
    source: sourceLang.value,
    target: targetLang.value,
  });
});

swapLangs.addEventListener('click', async () => {
  const tmp = sourceLang.value;
  sourceLang.value = targetLang.value;
  targetLang.value = tmp;
  await invoke('set_languages', {
    source: sourceLang.value,
    target: targetLang.value,
  });
});

testBtn.addEventListener('click', async () => {
  const text = testInput.value.trim();
  if (!text) return;

  testBtn.disabled = true;
  testBtn.textContent = 'Translating...';
  testResult.textContent = '';
  testResult.classList.add('visible');

  try {
    const resp = await invoke('test_translation', {
      request: {
        text,
        source_lang: sourceLang.value,
        target_lang: targetLang.value,
      },
    });
    testResult.textContent = resp.translation || '(empty result)';
  } catch (e) {
    testResult.textContent = 'Error: ' + e;
  } finally {
    testBtn.disabled = false;
    testBtn.textContent = 'Translate';
  }
});

// --- Start ---
init();
