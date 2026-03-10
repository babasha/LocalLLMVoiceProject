const { invoke } = window.__TAURI__.core;

// --- State ---
let state = {
  isRunning: false,
  mode: 'passthrough',
};

// --- DOM ---
const statusBadge = document.getElementById('statusBadge');
const startBtn = document.getElementById('startBtn');
const sourceLang = document.getElementById('sourceLang');
const targetLang = document.getElementById('targetLang');
const swapLangs = document.getElementById('swapLangs');
const deviceList = document.getElementById('deviceList');
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
}

async function refreshStatus() {
  try {
    const status = await invoke('get_status');
    state.isRunning = status.is_running;
    state.mode = status.mode;

    // Update UI
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
    startBtn.textContent = 'Stop';
    startBtn.classList.add('running');
  } else {
    statusBadge.textContent = 'Stopped';
    statusBadge.classList.remove('running');
    startBtn.textContent = 'Start';
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
    deviceList.innerHTML = '';

    if (devices.length === 0) {
      deviceList.innerHTML = '<span class="muted">No devices found</span>';
      return;
    }

    devices.forEach(dev => {
      const el = document.createElement('div');
      el.className = 'device-item' + (dev.is_default ? ' default' : '');
      el.innerHTML = `
        <span>${dev.name}</span>
        <span class="device-type ${dev.is_input ? '' : 'output'}">${dev.is_input ? 'IN' : 'OUT'}</span>
      `;
      deviceList.appendChild(el);
    });
  } catch (e) {
    deviceList.innerHTML = '<span class="muted">Failed to load devices</span>';
  }
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
