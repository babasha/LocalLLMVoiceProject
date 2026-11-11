# Voice Translator

Голосовой переводчик реального времени с полностью локальной обработкой. Говоришь по-русски — звучит английский, причём перевод начинает звучать ещё до конца фразы. Речь собеседника (звук системы) переводится в субтитры. Всё работает на твоём GPU: без интернета и облачных API.

```
Микрофон / звук системы → VAD → STT → LLM-перевод → TTS / субтитры
```

- **Синхронный режим**: частичные распознавания (LocalAgreement-2) переводятся инкрементально, уже озвученные слова никогда не переписываются.
- **Три режима**: `full` (ты говоришь → перевод голосом), `listen` (собеседник → субтитры), `duplex` (оба сразу, для созвонов).
- **Виртуальный микрофон**: перевод можно вывести в VB-CABLE, и собеседник в Telegram/Discord услышит английский.
- **Десктоп-приложение** для Windows (трей, настройки, менеджер моделей) и CLI.

## Быстрый старт

### Windows (нативно, GPU)

Разовая установка тулчейна через `winget`:

| Что | Пакет |
|---|---|
| Rust (MSVC) | `Rustlang.Rustup` |
| VS Build Tools 2022 + workload C++ (`Microsoft.VisualStudio.Workload.VCTools`) | `Microsoft.VisualStudio.2022.BuildTools` |
| CMake | `Kitware.CMake` |
| CUDA Toolkit 12.8+ (для RTX 50xx — 13.x) | `Nvidia.CUDA` |
| LLVM (libclang для bindgen) | `LLVM.LLVM` |

```bat
git clone https://github.com/babasha/LocalLLMVoiceProject.git
cd LocalLLMVoiceProject
build_win.bat                      :: сборка (MSVC + CUDA, генератор Ninja)
scripts\download_models.ps1        :: модели для режима full (-Mode listen|duplex, -All)
ui.bat                             :: десктоп-приложение
run_win.bat                        :: или полный пайплайн в консоли
```

Проверить окружение: `scripts\check_cuda.ps1`. Модели можно скачать и из приложения (вкладка «Модели»).

### Linux

```bash
git clone https://github.com/babasha/LocalLLMVoiceProject.git
cd LocalLLMVoiceProject
./setup.sh     # зависимости (apt), модели для режима full (~3.1 GB), сборка
./run.sh       # микрофон → перевод → голос
```

Нужны NVIDIA GPU и CUDA Toolkit 12.2+. Режимы `listen`/`duplex` используют WASAPI loopback и на Linux не проверялись; десктоп-приложение — только Windows.

## Требования

| Что | Минимум | Рекомендуется |
|---|---|---|
| ОС | Windows 10 / Ubuntu 22.04 | Windows 11 / Ubuntu 24.04 |
| GPU | NVIDIA, 4 GB VRAM | 6+ GB VRAM |
| RAM | 8 GB | 16 GB |
| Rust | 1.75+ | latest stable |
| CMake | 3.25+ | 3.28+ |

## Режимы

| Режим | Что делает | STT |
|---|---|---|
| `full` | Ты говоришь в микрофон → перевод звучит (колонки или виртуальный микрофон) | GigaAM-v3 (RU) |
| `listen` | Звук собеседника/видео захватывается loopback'ом с устройства вывода → субтитры с переводом | Parakeet TDT 0.6B v3 (EN + 24 европейских языка) |
| `duplex` | Оба канала одновременно. Одна LLM на GPU, вызовы двух каналов сериализуются | обе |
| `stt` | Только распознавание, без перевода | GigaAM-v3 |
| `passthrough` | Микрофон → колонки, проверка аудио | — |

```bat
run_win.bat --mode duplex --tts-output "CABLE Input" --input-device "Headset"
```

Параметры CLI (`voice-translator --help`):

| Параметр | Значение |
|---|---|
| `--mode` | `full` \| `listen` \| `duplex` \| `stt` \| `passthrough` |
| `--config` | путь к TOML (по умолчанию `config/default.toml`) |
| `--list-devices` [`--tsv`] | список аудиоустройств |
| `--source-lang`, `--target-lang` | языки (по умолчанию `ru` → `en`) |
| `--input-device` | микрофон (подстрока имени, пусто = по умолчанию) |
| `--loopback-device` | устройство вывода, чей звук слушаем в `listen`/`duplex` |
| `--tts-output` | куда озвучивать перевод |
| `--speak true\|false` | озвучивать ли перевод |
| `--llm-model` | путь к GGUF |
| `--silence-ms` | пауза конца фразы для VAD |

Переменные окружения: `VOICE_PERF=1` — замеры STT/LLM в stderr; `VOICE_LLAMA_LOGS=1` — логи llama.cpp; `RUST_LOG=debug` — подробный лог; `VOICE_TTS_OUTPUT` / `VOICE_TTS_SPEAK` / `VOICE_TTS_ENGINE` — переопределение секции `[tts]`.

### Виртуальный микрофон (Telegram, Discord, Zoom)

Установи [VB-CABLE](https://vb-audio.com/Cable/), выбери озвучку в `CABLE Input` (`--tts-output "CABLE Input"` или в настройках приложения), а в мессенджере укажи микрофоном `CABLE Output`.

Если перевод озвучивается на то же устройство, с которого идёт loopback, канал собеседника на время озвучки глушится (*echo gate*), иначе переводчик услышал бы сам себя. Лучше наушники или VB-CABLE.

## Десктоп-приложение (`ui.bat`)

Окно на WinForms (`ui_win.ps1`, без зависимостей), живёт в системном трее:

- **Перевод** — панели «Я» и «Собеседник». Слова, которые уже озвучены и не изменятся (*committed*), показаны ярко; хвост, который ещё может перестроиться (*draft*), — приглушённо. Ниже — история фраз; при отставании перевода у панели появляется предупреждение.
- **Настройки** — режим, микрофон, устройство для loopback, куда озвучивать (колонки / `CABLE Input` / без голоса), LLM (любой `.gguf` из `models/` или LM Studio), пауза конца фразы, язык интерфейса (RU/EN). Хранятся в `%APPDATA%\VoiceTranslator\settings.json`.
- **Модели** — статус каждой модели из `config/models.json` для выбранного режима и скачивание недостающих. Без нужных моделей пайплайн не стартует, а открывает эту вкладку.

Приложение запускает `voice-translator.exe` с `VOICE_UI=1` и читает из его stdout записи `READY / SRC / TR / FINAL / WARN`.

## Модели

| Компонент | Модель | Размер | Режимы | Источник |
|---|---|---|---|---|
| VAD | Silero VAD v5 | 2 MB | все | [snakers4/silero-vad](https://github.com/snakers4/silero-vad) |
| STT (ты) | GigaAM-v3 e2e CTC, INT8 | 320 MB | full, duplex | [Smirnov75/GigaAM-v3-sherpa-onnx](https://huggingface.co/Smirnov75/GigaAM-v3-sherpa-onnx) |
| STT (собеседник) | Parakeet TDT 0.6B v3, INT8 | 490 MB | listen, duplex | [k2-fsa/sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx/releases/tag/asr-models) |
| Перевод | Qwen3.5-4B Q4_K_M | 2.7 GB | все | [unsloth/Qwen3.5-4B-GGUF](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF) |
| TTS | Piper en_US lessac (VITS) | 67 MB | full, duplex | [k2-fsa/sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models) |

Манифест — `config/models.json`; его используют и приложение, и `scripts/download_models.ps1`. LLM можно заменить любой GGUF-моделью с шаблоном ChatML или Gemma (например, Gemma 4 E2B — быстрее на токен): шаблон определяется по имени файла или задаётся `prompt_style`.

## Как это работает

```
┌────────────┐   ┌────────┐   ┌──────────────┐   ┌──────────────┐   ┌──────────┐
│ Захват     │──→│  VAD   │──→│     STT      │──→│   Перевод    │──→│   TTS    │
│ (cpal)     │   │ Silero │   │ sherpa-onnx  │   │  llama.cpp   │   │  Piper   │
└────────────┘   └────────┘   └──────────────┘   └──────────────┘   └──────────┘
 родной формат    512 сэмплов   partial каждые     продолжение        поток в
 → 16 кГц моно    на кадр       0.4 с + final      озвученного        колонки/CABLE
```

**Потоки** (на каждый канал — «я» и «собеседник»):

- **Аудио-callback** (cpal, real-time): сводит в моно, понижает частоту до 16 кГц и кладёт сэмплы в lock-free кольцевой буфер (`rtrb`) — без аллокаций и блокировок.
- **`stt-<канал>`**: читает буфер, кормит Silero VAD, распознаёт. Отправляет в канал `crossbeam` события `Partial` (стабильный префикс) и `Final` (законченная фраза).
- **`translate-<канал>`**: вычитывает все накопившиеся события, переводит и отдаёт текст в UI и TTS.
- **`tts-piper`**: синтезирует очередь фрагментов и проигрывает через своё устройство вывода.

LLM грузится в фоне, пока поднимаются захват и STT. Модель одна на оба канала; вызовы сериализуются мьютексом.

**Синхронный перевод.** Пока человек говорит, растущий буфер фразы перераспознаётся каждые 0.4 с. Стабильным считается префикс, совпавший в двух подряд распознаваниях (LocalAgreement-2) — он сразу уходит на перевод. Перевод инкрементальный: уже озвученный английский текст подставляется как начало ответа модели, и она *продолжает* его, а не переводит заново. Поэтому модель не может «передумать» насчёт слов, которые слушатель уже услышал. Последние два слова каждого частичного перевода остаются черновиком и озвучиваются, когда подтвердятся. Декодирование жадное (`temperature = 0`), чтобы соседние частичные переводы не «дрожали».

**Надёжность:**

- **Фильтр мусора STT** (`stt/filter.rs`): RMS-гейт (тихие сегменты не распознаются), отбрасывание результатов без букв, схлопывание зацикливаний («да да да да да» → «да да да»), список известных галлюцинаций («Продолжение следует…», «Thanks for watching!») + свой `blocklist` в конфиге.
- **Отставание перевода**: финальные фразы не теряются — если их накопилось несколько, они переводятся одним вызовом (`merge_backlog`); при задержке больше `lag_warn_ms` — предупреждение; фразы старше `drop_after_ms` показываются без перевода и не озвучиваются (поздняя речь в живом разговоре хуже, чем никакой).
- **Предел длины сегмента**: при непрерывной речи сегмент принудительно закрывается через `max_speech_duration_ms + 1 с`.
- **Loopback без звука**: WASAPI loopback не присылает пакеты, пока ничего не играет, — после 200 мс в VAD подаётся синтетическая тишина, иначе фраза не завершалась бы.
- **Pre-roll 600 мс**: первые слова фразы не теряются, пока VAD «просыпается».
- Текст, который уже на целевом языке (определяется по письменности), не отправляется в LLM.

## Конфигурация

Всё в `config/default.toml`; относительные пути разрешаются от корня проекта. Основное:

```toml
[languages]
source = "ru"                 # что говоришь ты
target = "en"                 # что слышит собеседник (его речь переводится обратно)

[vad]
silence_duration_ms = 250     # пауза конца фразы: меньше — быстрее, но режет на коротких паузах
max_speech_duration_ms = 7000 # длинный монолог режется на части

[translation]
model_path = "models/Qwen3.5-4B-Q4_K_M.gguf"
prompt_style = ""             # "chatml" | "gemma" | "" (по имени файла)
n_gpu_layers = 99             # уменьши, если не хватает VRAM
temperature = 0.0             # жадное декодирование — стабильные частичные переводы

[tts]
engine = "piper"              # или "sapi" — встроенный голос Windows
speak = true
output_device = ""            # "CABLE Input" — виртуальный микрофон
rate = 2                      # скорость речи

[pipeline]
lag_warn_ms = 2500
merge_backlog = true
drop_after_ms = 12000         # 0 = никогда не отбрасывать
```

Секции `[stt]` / `[stt_listen]` задают распознаватель (`engine = "nemo_ctc" | "nemo_transducer" | "whisper"`) и его фильтр.

## Нюансы Windows

- **OneDrive.** Если проект лежит в OneDrive, отметь папку `models` «Всегда хранить на этом устройстве»: placeholder-файлы llama.cpp прочитать не может (`failed to read magic`). `build_win.bat` в этом случае сам выносит артефакты сборки в `C:\cargo-target\voice-translator` (переопределяется через `CARGO_TARGET_DIR`).
- **sherpa-onnx.** Нативные библиотеки лежат в `libs/sherpa-onnx-win` (релиз k2-fsa `win-x64-shared-MD-Release`; `onnxruntime.lib` сгенерирована из DLL). DLL должны находиться раньше системной `onnxruntime.dll` из `System32` — скрипты запуска добавляют папку в `PATH`.
- **CUDA 13.** Рантайм-DLL (`cublas64_13` и др.) лежат в `bin\x64`. llama.cpp собирается генератором Ninja: генератор Visual Studio спотыкается на CUDA-таргетах. Чтобы ускорить сборку, можно собрать ядра только под свою архитектуру: `set CMAKE_CUDA_ARCHITECTURES=120` (RTX 50xx).
- **`llama-cpp-2` ≥ 0.1.146** нужен для архитектур `qwen35` и `gemma4`.
- **Аудио** захватывается в родном формате устройства (например, 48 кГц стерео) и приводится к 16 кГц моно на лету: прямой запрос 16 кГц многие устройства не поддерживают.

## Структура проекта

```
├── build_win.bat / run_win.bat    # сборка и запуск в консоли (Windows)
├── ui.bat / ui_win.ps1            # десктоп-приложение (Windows)
├── setup.sh / run.sh              # установка и запуск (Linux)
├── config/
│   ├── default.toml               # конфигурация
│   └── models.json                # манифест моделей
├── scripts/
│   ├── download_models.ps1        # скачивание моделей
│   └── check_cuda.ps1             # проверка окружения
├── libs/                          # нативные библиотеки sherpa-onnx (Windows, Linux)
├── models/                        # модели (скачиваются, не в git)
└── crates/voice-core/
    ├── examples/test_llm.rs       # бенчмарк перевода
    └── src/
        ├── main.rs                # CLI
        ├── config.rs              # загрузка TOML
        ├── audio/                 # устройства, захват (+ loopback), воспроизведение
        ├── vad/detector.rs        # Silero VAD
        ├── stt/                   # распознаватель (sherpa-onnx) и фильтр мусора
        ├── translation/           # LLM (llama.cpp) и промпты
        ├── tts/                   # Piper, SAPI, echo gate
        └── pipeline/              # оркестратор потоков
```

## Разработка

```bash
cargo test -p voice-core                              # юнит-тесты
VOICE_PERF=1 cargo run --release --example test_llm   # скорость перевода (ток/с)
```

На Windows запускай из окружения `build_win.bat` (MSVC, CUDA, `SHERPA_ONNX_LIB_DIR`); аргументы скрипта передаются в `cargo build`.

## Лицензия

[MIT](LICENSE). В репозитории лежат готовые сборки [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) (Apache-2.0) и [ONNX Runtime](https://github.com/microsoft/onnxruntime) (MIT). Модели распространяются на условиях своих лицензий.
